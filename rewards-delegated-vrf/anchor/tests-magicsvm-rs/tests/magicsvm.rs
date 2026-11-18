//! Rewards distributor with one SPL reward that every draw wins. Draws are ER
//! randomness requests whose VRF callback picks the reward, commits the reward
//! list and, with a transfer lookup table, pays out on base through a
//! post-commit `transfer_spl_token` action. NFT rewards need the Metaplex
//! programs, which these tests don't load.

use {
    anchor_lang::{prelude::Pubkey, system_program, AccountDeserialize, Id},
    anchor_spl::associated_token::get_associated_token_address,
    ephemeral_rollups_sdk::pda::ephemeral_balance_pda_from_payer,
    magicsvm::{MagicSVM, TransactionTarget},
    rewards_delegated_vrf::{
        accounts, constants, instruction,
        state::{Reward, RewardsList},
        ACTION_ESCROW_INDEX,
    },
    solana_account::Account,
    solana_clock::Clock,
    solana_instruction::{AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_native_token::LAMPORTS_PER_SOL,
    solana_signer::Signer,
    test_utils::{
        as_address, as_pubkey, boot, client_ix,
        delegate_buffer_pda_from_delegated_account_and_owner_program,
        delegation_metadata_pda_from_delegated_account,
        delegation_record_pda_from_delegated_account, is_delegated, program_so_path, read_u64_le,
        send_ix, sign_tx, DelegationProgram, MagicProgram, MAGIC_CONTEXT_ID,
    },
};

const PROGRAM_ID: Pubkey = rewards_delegated_vrf::ID;
const VRF_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz");
const DEFAULT_EPHEMERAL_QUEUE: Pubkey =
    Pubkey::from_str_const("5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc");
const SLOT_HASHES: Pubkey = Pubkey::from_str_const("SysvarS1otHashes111111111111111111111111111");
const BPF_LOADER_UPGRADEABLE: Pubkey =
    Pubkey::from_str_const("BPFLoaderUpgradeab1e11111111111111111111111");
const TOKEN_PROGRAM: Pubkey = anchor_spl::token::ID;

const REWARD_AMOUNT: u64 = 7;
const REDEMPTION_LIMIT: u64 = 100;
const DISTRIBUTOR_BALANCE: u64 = 1_000;

struct Rewards {
    svm: MagicSVM,
    admin: Keypair,
    user: Pubkey,
    mint: Pubkey,
    reward_distributor: Pubkey,
    whitelist_distributor: Pubkey,
    reward_list: Pubkey,
    transfer_lookup_table: Pubkey,
    draws: u8,
}

fn ix(
    data: impl anchor_lang::InstructionData,
    accounts: impl anchor_lang::ToAccountMetas,
) -> Instruction {
    client_ix(as_address(PROGRAM_ID), data, accounts)
}

/// Writes an initialized SPL token account (0-decimal mint).
fn set_token_account(
    svm: &mut MagicSVM,
    address: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
) {
    let mut data = vec![0u8; 165];
    data[..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(owner.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1; // AccountState::Initialized
    set_token_program_account(svm, address, data);
}

fn set_token_program_account(svm: &mut MagicSVM, address: Pubkey, data: Vec<u8>) {
    let lamports = svm.minimum_balance_for_rent_exemption(data.len());
    svm.set_account(
        as_address(address),
        Account {
            lamports,
            data,
            owner: as_address(TOKEN_PROGRAM),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

fn token_balance(svm: &MagicSVM, account: &Pubkey) -> Option<u64> {
    svm.get_account(&as_address(*account))
        .map(|account| read_u64_le(&account.data, 64))
}

/// Sets up the distributor, its reward list and the SPL reward, then
/// delegates the reward list and funds both action escrows. Without `payouts`
/// the lookup table is empty, so a draw only records the win.
fn setup(payouts: bool) -> Rewards {
    let admin = Keypair::new();
    let mut svm = boot(
        env!("CARGO_MANIFEST_DIR"),
        "rewards_delegated_vrf",
        as_address(PROGRAM_ID),
        &admin,
        10 * LAMPORTS_PER_SOL,
    );
    let admin_key = as_pubkey(&admin.pubkey());

    // `initialize_transfer_lookup_table` is gated on the upgrade authority, so
    // redeploy the program as upgradeable with the admin as authority.
    let elf = std::fs::read(program_so_path(
        env!("CARGO_MANIFEST_DIR"),
        "rewards_delegated_vrf",
    ))
    .unwrap();
    svm.add_program_with_loader(
        as_address(PROGRAM_ID),
        &elf,
        as_address(BPF_LOADER_UPGRADEABLE),
    )
    .unwrap();
    let (program_data, _) =
        Pubkey::find_program_address(&[PROGRAM_ID.as_ref()], &BPF_LOADER_UPGRADEABLE);
    let mut program_data_account = svm.get_account(&as_address(program_data)).unwrap();
    program_data_account.data[12] = 1; // upgrade_authority_address: Some(admin)
    program_data_account.data[13..45].copy_from_slice(admin_key.as_ref());
    svm.set_account(as_address(program_data), program_data_account)
        .unwrap();

    let (reward_distributor, _) = Pubkey::find_program_address(
        &[constants::REWARD_DISTRIBUTOR_SEED, admin_key.as_ref()],
        &PROGRAM_ID,
    );
    let (whitelist_distributor, _) = Pubkey::find_program_address(
        &[
            constants::WHITELIST_DISTRIBUTOR_SEED,
            reward_distributor.as_ref(),
        ],
        &PROGRAM_ID,
    );
    let (reward_list, _) = Pubkey::find_program_address(
        &[constants::REWARD_LIST_SEED, reward_distributor.as_ref()],
        &PROGRAM_ID,
    );
    let (transfer_lookup_table, _) =
        Pubkey::find_program_address(&[constants::TRANSFER_LOOKUP_TABLE_SEED], &PROGRAM_ID);

    // A 0-decimal SPL mint; the distributor's ATA holds the inventory.
    let mint = Pubkey::new_unique();
    let mut mint_data = vec![0u8; 82];
    mint_data[36..44].copy_from_slice(&DISTRIBUTOR_BALANCE.to_le_bytes());
    mint_data[45] = 1; // is_initialized
    set_token_program_account(&mut svm, mint, mint_data);
    let distributor_ata = get_associated_token_address(&reward_distributor, &mint);
    set_token_account(
        &mut svm,
        distributor_ata,
        mint,
        reward_distributor,
        DISTRIBUTOR_BALANCE,
    );

    let lookup_accounts = if payouts {
        vec![
            TOKEN_PROGRAM,
            anchor_spl::associated_token::ID,
            system_program::ID,
            anchor_spl::metadata::ID,
            constants::SYSVAR_INSTRUCTIONS_ID,
            constants::MPL_TOKEN_AUTH_RULES_ID,
        ]
    } else {
        vec![]
    };
    let base_ixs = [
        (
            ix(
                instruction::InitializeRewardDistributor { admins: vec![] },
                accounts::InitializeRewardDistributor {
                    initializer: admin_key,
                    reward_distributor,
                    whitelist_distributor,
                    system_program: system_program::ID,
                },
            ),
            "initialize_reward_distributor",
        ),
        (
            ix(
                instruction::SetRewardList {
                    start_timestamp: Some(0),
                    end_timestamp: Some(i64::MAX),
                    global_range_min: Some(1),
                    global_range_max: Some(100),
                },
                accounts::SetRewardList {
                    admin: admin_key,
                    reward_distributor,
                    reward_list,
                    system_program: system_program::ID,
                },
            ),
            "set_reward_list",
        ),
        (
            ix(
                instruction::AddReward {
                    reward_name: "Tokens".into(),
                    reward_amount: Some(REWARD_AMOUNT),
                    draw_range_min: Some(1),
                    draw_range_max: Some(100),
                    redemption_limit: Some(REDEMPTION_LIMIT),
                },
                accounts::AddReward {
                    admin: admin_key,
                    reward_distributor,
                    reward_list,
                    mint,
                    token_account: distributor_ata,
                    metadata: None,
                },
            ),
            "add_reward",
        ),
        (
            ix(
                instruction::InitializeTransferLookupTable { lookup_accounts },
                accounts::InitializeTransferLookupTable {
                    authority: admin_key,
                    program: PROGRAM_ID,
                    program_data,
                    transfer_lookup_table,
                    system_program: system_program::ID,
                },
            ),
            "initialize_transfer_lookup_table",
        ),
    ];
    for (instruction, label) in base_ixs {
        send_ix(
            &mut svm,
            TransactionTarget::Base,
            &[&admin],
            instruction,
            label,
        );
    }

    let mut delegate = ix(
        instruction::DelegateRewardList {},
        accounts::DelegateRewardList {
            admin: admin_key,
            reward_distributor,
            buffer_reward_list: delegate_buffer_pda_from_delegated_account_and_owner_program(
                &reward_list,
                &PROGRAM_ID,
            ),
            delegation_record_reward_list: delegation_record_pda_from_delegated_account(
                &reward_list,
            ),
            delegation_metadata_reward_list: delegation_metadata_pda_from_delegated_account(
                &reward_list,
            ),
            reward_list,
            owner_program: PROGRAM_ID,
            delegation_program: DelegationProgram::id(),
            system_program: system_program::ID,
        },
    );
    delegate
        .accounts
        .push(AccountMeta::new_readonly(svm.validator_identity(), false));
    send_ix(
        &mut svm,
        TransactionTarget::Base,
        &[&admin],
        delegate,
        "delegate_reward_list",
    );
    assert!(is_delegated(&svm, &as_address(reward_list)));

    // Payout actions pay the recipient's ATA rent from the source's escrow.
    for source in [reward_distributor, whitelist_distributor] {
        let escrow = ephemeral_balance_pda_from_payer(&source, ACTION_ESCROW_INDEX);
        svm.airdrop(&as_address(escrow), LAMPORTS_PER_SOL).unwrap();
    }

    Rewards {
        svm,
        admin,
        user: Pubkey::new_unique(),
        mint,
        reward_distributor,
        whitelist_distributor,
        reward_list,
        transfer_lookup_table,
        draws: 0,
    }
}

impl Rewards {
    fn reward(&self, target: TransactionTarget) -> Reward {
        let data = self
            .svm
            .get_account_for(target, &as_address(self.reward_list))
            .unwrap()
            .data;
        RewardsList::try_deserialize(&mut &data[..])
            .unwrap()
            .rewards[0]
            .clone()
    }

    fn magic_fee_vault(&self) -> Pubkey {
        Pubkey::find_program_address(
            &[
                b"magic-fee-vault",
                as_pubkey(&self.svm.validator_identity()).as_ref(),
            ],
            &DelegationProgram::id(),
        )
        .0
    }

    fn distributor_ata(&self) -> Pubkey {
        get_associated_token_address(&self.reward_distributor, &self.mint)
    }

    fn user_ata(&self) -> Pubkey {
        get_associated_token_address(&self.user, &self.mint)
    }

    /// Sends `instruction` to the ER signed by `signer`, returning the error
    /// and logs on failure.
    fn send_er(&mut self, signer: &Keypair, instruction: Instruction) -> Result<(), String> {
        let tx = sign_tx(
            &[signer],
            &[instruction],
            self.svm.latest_blockhash_for(TransactionTarget::Ephemeral),
        );
        self.svm
            .send_transaction_to(TransactionTarget::Ephemeral, tx)
            .map(drop)
            .map_err(|failed| format!("{:?}\n{}", failed.err, failed.meta.logs.join("\n")))
    }

    fn set_whitelist(&mut self, whitelist: Vec<Pubkey>) {
        let set_whitelist = ix(
            instruction::SetWhitelist { whitelist },
            accounts::SetWhitelist {
                admin: as_pubkey(&self.admin.pubkey()),
                reward_distributor: self.reward_distributor,
            },
        );
        send_ix(
            &mut self.svm,
            TransactionTarget::Base,
            &[&self.admin],
            set_whitelist,
            "set_whitelist",
        );
    }

    /// Requests randomness on the ER as `signer`. Each request uses a new
    /// client seed, so repeated draws are distinct transactions.
    fn request(&mut self, signer: &Keypair) -> Result<(), String> {
        self.draws += 1;
        let (program_identity, _) = Pubkey::find_program_address(&[b"identity"], &PROGRAM_ID);
        let request = ix(
            instruction::RequestRandomReward {
                client_seed: self.draws,
            },
            accounts::RequestRandomReward {
                user: self.user,
                admin: as_pubkey(&signer.pubkey()),
                reward_distributor: self.reward_distributor,
                reward_list: self.reward_list,
                transfer_lookup_table: self.transfer_lookup_table,
                oracle_queue: DEFAULT_EPHEMERAL_QUEUE,
                delegation_record_reward_list: delegation_record_pda_from_delegated_account(
                    &self.reward_list,
                ),
                program_identity,
                vrf_program: VRF_PROGRAM_ID,
                slot_hashes: SLOT_HASHES,
                system_program: system_program::ID,
            },
        );
        self.send_er(signer, request)
    }

    /// Advances a slot, which runs the pending callback.
    fn fulfill(&mut self) {
        let slot = self.svm.get_sysvar::<Clock>().slot;
        let results = self.svm.warp_to_slot(slot + 1);
        assert_eq!(results.len(), 1);
        if let Err(failed) = &results[0] {
            panic!(
                "callback failed: {:?}\nlogs: {:#?}",
                failed.err, failed.meta.logs
            );
        }
    }

    fn draw(&mut self) {
        let admin = self.admin.insecure_clone();
        self.request(&admin).unwrap();
        self.fulfill();
    }

    fn admin_transfer(&mut self, amount: u64) -> Result<(), String> {
        let admin_transfer = ix(
            instruction::AdminTransfer { amount },
            accounts::AdminTransfer {
                admin: as_pubkey(&self.admin.pubkey()),
                reward_distributor: self.reward_distributor,
                reward_list: self.reward_list,
                transfer_lookup_table: self.transfer_lookup_table,
                mint: self.mint,
                source_token_account: self.distributor_ata(),
                user: self.user,
                delegation_record_reward_list: delegation_record_pda_from_delegated_account(
                    &self.reward_list,
                ),
                magic_fee_vault: self.magic_fee_vault(),
                magic_program: MagicProgram::id(),
                magic_context: MAGIC_CONTEXT_ID,
            },
        );
        let admin = self.admin.insecure_clone();
        self.send_er(&admin, admin_transfer)
    }

    fn undelegate(&mut self) {
        let undelegate = ix(
            instruction::UndelegateRewardList {},
            accounts::UndelegateRewardList {
                payer: as_pubkey(&self.admin.pubkey()),
                reward_distributor: self.reward_distributor,
                reward_list: self.reward_list,
                magic_program: MagicProgram::id(),
                magic_context: MAGIC_CONTEXT_ID,
            },
        );
        send_ix(
            &mut self.svm,
            TransactionTarget::Ephemeral,
            &[&self.admin],
            undelegate,
            "undelegate_reward_list",
        );
        assert!(!is_delegated(&self.svm, &as_address(self.reward_list)));
    }
}

#[test]
fn vrf_callback_on_the_er_records_the_win_and_commits() {
    let mut rewards = setup(false);
    let admin = rewards.admin.insecure_clone();
    rewards.request(&admin).unwrap();
    assert_eq!(
        rewards
            .reward(TransactionTarget::Ephemeral)
            .redemption_count,
        0,
        "pending until the next slot"
    );
    rewards.fulfill();
    assert_eq!(
        rewards
            .reward(TransactionTarget::Ephemeral)
            .redemption_count,
        1
    );
    assert_eq!(token_balance(&rewards.svm, &rewards.user_ata()), None);

    rewards.undelegate();
    assert_eq!(rewards.reward(TransactionTarget::Base).redemption_count, 1);
}

#[test]
fn vrf_callback_pays_the_spl_reward_on_base() {
    let mut rewards = setup(true);
    rewards.draw();

    assert_eq!(
        token_balance(&rewards.svm, &rewards.user_ata()),
        Some(REWARD_AMOUNT)
    );
    assert_eq!(
        token_balance(&rewards.svm, &rewards.distributor_ata()),
        Some(DISTRIBUTOR_BALANCE - REWARD_AMOUNT)
    );
    assert_eq!(
        rewards.reward(TransactionTarget::Base).redemption_count,
        1,
        "the callback commits the reward list"
    );
    assert!(is_delegated(&rewards.svm, &as_address(rewards.reward_list)));
    rewards.undelegate();
}

#[test]
fn every_draw_pays_until_the_reward_is_exhausted() {
    let mut rewards = setup(true);
    for _ in 0..REDEMPTION_LIMIT {
        rewards.draw();
    }
    assert_eq!(
        token_balance(&rewards.svm, &rewards.user_ata()),
        Some(REDEMPTION_LIMIT * REWARD_AMOUNT)
    );

    // Exhausted: the callback succeeds but pays nothing.
    rewards.draw();
    assert_eq!(
        rewards.reward(TransactionTarget::Base).redemption_count,
        REDEMPTION_LIMIT
    );
    assert_eq!(
        token_balance(&rewards.svm, &rewards.user_ata()),
        Some(REDEMPTION_LIMIT * REWARD_AMOUNT)
    );
}

#[test]
fn only_admins_and_whitelisted_signers_can_request() {
    let mut rewards = setup(false);
    let stranger = Keypair::new();
    rewards
        .svm
        .airdrop(&stranger.pubkey(), LAMPORTS_PER_SOL)
        .unwrap();

    let err = rewards.request(&stranger).unwrap_err();
    assert!(err.contains("ConstraintRaw"), "{err}");

    rewards.set_whitelist(vec![as_pubkey(&stranger.pubkey())]);
    rewards.request(&stranger).unwrap();
    rewards.fulfill();
    assert_eq!(
        rewards
            .reward(TransactionTarget::Ephemeral)
            .redemption_count,
        1
    );
}

#[test]
fn admin_transfer_pays_only_from_uncommitted_inventory() {
    let mut rewards = setup(true);
    // The reward holds back REDEMPTION_LIMIT * REWARD_AMOUNT for future wins.
    let free = DISTRIBUTOR_BALANCE - REDEMPTION_LIMIT * REWARD_AMOUNT;

    rewards.admin_transfer(free).unwrap();
    assert_eq!(token_balance(&rewards.svm, &rewards.user_ata()), Some(free));

    let err = rewards.admin_transfer(1).unwrap_err();
    assert!(err.contains("InsufficientTokenBalanceForReward"), "{err}");
}

#[test]
fn whitelist_transfer_pays_from_the_whitelist_bag() {
    let mut rewards = setup(true);
    let whitelisted = Keypair::new();
    rewards
        .svm
        .airdrop(&whitelisted.pubkey(), LAMPORTS_PER_SOL)
        .unwrap();
    rewards.set_whitelist(vec![as_pubkey(&whitelisted.pubkey())]);
    let bag = get_associated_token_address(&rewards.whitelist_distributor, &rewards.mint);
    set_token_account(
        &mut rewards.svm,
        bag,
        rewards.mint,
        rewards.whitelist_distributor,
        50,
    );

    let whitelist_transfer = ix(
        instruction::WhitelistTransfer { amount: 20 },
        accounts::WhitelistTransfer {
            signer: as_pubkey(&whitelisted.pubkey()),
            reward_distributor: rewards.reward_distributor,
            whitelist_distributor: rewards.whitelist_distributor,
            reward_list: rewards.reward_list,
            transfer_lookup_table: rewards.transfer_lookup_table,
            mint: rewards.mint,
            source_token_account: bag,
            user: rewards.user,
            delegation_record_reward_list: delegation_record_pda_from_delegated_account(
                &rewards.reward_list,
            ),
            magic_fee_vault: rewards.magic_fee_vault(),
            magic_program: MagicProgram::id(),
            magic_context: MAGIC_CONTEXT_ID,
        },
    );
    rewards.send_er(&whitelisted, whitelist_transfer).unwrap();

    assert_eq!(token_balance(&rewards.svm, &rewards.user_ata()), Some(20));
    assert_eq!(token_balance(&rewards.svm, &bag), Some(30));
    assert_eq!(
        token_balance(&rewards.svm, &rewards.distributor_ata()),
        Some(DISTRIBUTOR_BALANCE)
    );
}

#[test]
fn update_and_remove_reward_on_the_er() {
    let mut rewards = setup(true);
    let admin = rewards.admin.insecure_clone();
    let admin_key = as_pubkey(&admin.pubkey());

    let update_reward = ix(
        instruction::UpdateReward {
            current_reward_name: "Tokens".into(),
            updated_reward_name: Some("Coins".into()),
            reward_amount: Some(5),
            draw_range_min: None,
            draw_range_max: None,
        },
        accounts::UpdateReward {
            admin: admin_key,
            reward_distributor: rewards.reward_distributor,
            reward_list: rewards.reward_list,
            mint: Some(rewards.mint),
            token_account: Some(rewards.distributor_ata()),
        },
    );
    rewards.send_er(&admin, update_reward).unwrap();
    let reward = rewards.reward(TransactionTarget::Ephemeral);
    assert_eq!((reward.name.as_str(), reward.reward_amount), ("Coins", 5));

    // Removing 10 redemptions returns 10 * 5 tokens to the destination.
    let destination = Pubkey::new_unique();
    let remove_reward = ix(
        instruction::RemoveReward {
            reward_name: "Coins".into(),
            mint_to_remove: Some(rewards.mint),
            redemption_amount: Some(10),
        },
        accounts::RemoveReward {
            admin: admin_key,
            reward_distributor: rewards.reward_distributor,
            reward_list: rewards.reward_list,
            transfer_lookup_table: rewards.transfer_lookup_table,
            destination,
            delegation_record_reward_list: delegation_record_pda_from_delegated_account(
                &rewards.reward_list,
            ),
            magic_fee_vault: rewards.magic_fee_vault(),
            magic_program: MagicProgram::id(),
            magic_context: MAGIC_CONTEXT_ID,
        },
    );
    rewards.send_er(&admin, remove_reward).unwrap();

    let destination_ata = get_associated_token_address(&destination, &rewards.mint);
    assert_eq!(token_balance(&rewards.svm, &destination_ata), Some(50));
    let committed = rewards.reward(TransactionTarget::Base);
    assert_eq!(
        (committed.name.as_str(), committed.redemption_limit),
        ("Coins", REDEMPTION_LIMIT - 10)
    );
}
