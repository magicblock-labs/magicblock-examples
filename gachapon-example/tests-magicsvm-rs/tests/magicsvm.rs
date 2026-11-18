//! Machine setup and the VRF pull request, in-process.
//!
//! The `consume_pull` callback mints through Metaplex Core, which MagicSVM doesn't
//! ship, so this suite stops at the randomness request.

use {
    anchor_lang::{prelude::Pubkey, system_program, AccountDeserialize},
    gachapon_example::{
        accounts, instruction, Machine, PendingPull, RewardTemplateInput, ASSET_SEED,
        DEFAULT_VRF_QUEUE, MACHINE_SEED, MPL_CORE_ID, PULL_SEED, SLOT_HASHES_ID,
        TREASURY_SEED, TREASURY_TOP_UP_LAMPORTS, UPDATE_AUTHORITY_SEED, VRF_IDENTITY_SEED,
        VRF_PROGRAM_ID,
    },
    magicsvm::{MagicSVM, TransactionTarget},
    solana_instruction::{error::InstructionError, Instruction},
    solana_keypair::Keypair,
    solana_native_token::LAMPORTS_PER_SOL,
    solana_signer::Signer,
    solana_system_interface::instruction as system_instruction,
    solana_transaction_error::TransactionError,
    test_utils::{as_address, as_pubkey, boot, client_ix, program_id_from_idl, send_ix, sign_tx},
};

const MACHINE_ID: u64 = 42;
/// What the VRF program charges for a high-priority request: `pull` sends request
/// tag 3, solana-vrf's `RequestHighPriorityRandomness`.
const VRF_HIGH_PRIORITY_LAMPORTS_COST: u64 = 800_000;
/// Anchor's `ConstraintHasOne`.
const CONSTRAINT_HAS_ONE: u32 = 2001;

fn program_id() -> Pubkey {
    as_pubkey(&program_id_from_idl(
        env!("CARGO_MANIFEST_DIR"),
        "gachapon_example.json",
    ))
}

fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &program_id()).0
}

fn ix(
    data: impl anchor_lang::InstructionData,
    accounts: impl anchor_lang::ToAccountMetas,
) -> Instruction {
    client_ix(as_address(program_id()), data, accounts)
}

fn rewards() -> [RewardTemplateInput; 4] {
    [
        (55, "Bronze Capsule", "bronze"),
        (30, "Silver Capsule", "silver"),
        (12, "Gold Capsule", "gold"),
        (3, "Mythic Capsule", "mythic"),
    ]
    .map(|(weight, name, tier)| RewardTemplateInput {
        weight,
        name: name.to_string(),
        uri: format!("https://example.com/gachapon/{tier}.json"),
    })
}

fn fetch<T: AccountDeserialize>(svm: &MagicSVM, address: Pubkey) -> T {
    let account = svm
        .get_account_for(TransactionTarget::Base, &as_address(address))
        .unwrap_or_else(|| panic!("missing {address}"));
    T::try_deserialize(&mut &account.data[..]).unwrap()
}

fn balance(svm: &MagicSVM, address: Pubkey) -> u64 {
    svm.get_balance(&as_address(address)).unwrap_or(0)
}

struct Setup {
    svm: MagicSVM,
    authority: Keypair,
    machine: Pubkey,
    treasury: Pubkey,
    update_authority: Pubkey,
}

/// Boots MagicSVM and runs `init` for `MACHINE_ID`.
fn init_machine() -> Setup {
    let authority = Keypair::new();
    let mut svm = boot(
        env!("CARGO_MANIFEST_DIR"),
        "gachapon_example",
        as_address(program_id()),
        &authority,
        10 * LAMPORTS_PER_SOL,
    );
    let authority_key = as_pubkey(&authority.pubkey());
    let machine = pda(&[
        MACHINE_SEED,
        authority_key.as_ref(),
        &MACHINE_ID.to_le_bytes(),
    ]);
    let treasury = pda(&[TREASURY_SEED, machine.as_ref()]);
    let update_authority = pda(&[UPDATE_AUTHORITY_SEED, machine.as_ref()]);
    send_ix(
        &mut svm,
        TransactionTarget::Base,
        &[&authority],
        ix(
            instruction::Init {
                machine_id: MACHINE_ID,
            },
            accounts::Init {
                authority: authority_key,
                machine,
                treasury,
                update_authority,
                system_program: system_program::ID,
            },
        ),
        "init",
    );
    Setup {
        svm,
        authority,
        machine,
        treasury,
        update_authority,
    }
}

fn upload_config(setup: &mut Setup) {
    send_ix(
        &mut setup.svm,
        TransactionTarget::Base,
        &[&setup.authority],
        ix(
            instruction::UploadConfig { rewards: rewards() },
            accounts::UploadConfig {
                authority: as_pubkey(&setup.authority.pubkey()),
                machine: setup.machine,
            },
        ),
        "upload_config",
    );
}

#[test]
fn init_creates_the_machine_and_funds_the_treasury() {
    let setup = init_machine();
    let machine: Machine = fetch(&setup.svm, setup.machine);
    assert_eq!(machine.authority, as_pubkey(&setup.authority.pubkey()));
    assert_eq!(machine.machine_id, MACHINE_ID);
    assert_eq!(machine.total_weight, 0);
    assert_eq!(machine.pull_count, 0);
    assert_eq!(balance(&setup.svm, setup.treasury), TREASURY_TOP_UP_LAMPORTS);
}

#[test]
fn upload_config_writes_four_weighted_templates() {
    let mut setup = init_machine();
    upload_config(&mut setup);

    let machine: Machine = fetch(&setup.svm, setup.machine);
    assert_eq!(machine.total_weight, 100);
    for (index, (template, input)) in machine.rewards.iter().zip(rewards()).enumerate() {
        assert_eq!(template.reward_id, index as u8);
        assert_eq!(template.weight, input.weight);
        assert_eq!(template.minted_count, 0);
        assert_eq!(template.name, input.name);
        assert_eq!(template.uri, input.uri);
    }
}

#[test]
fn only_the_authority_can_upload_config() {
    let mut setup = init_machine();
    let stranger = Keypair::new();
    send_ix(
        &mut setup.svm,
        TransactionTarget::Base,
        &[&setup.authority],
        system_instruction::transfer(
            &setup.authority.pubkey(),
            &stranger.pubkey(),
            LAMPORTS_PER_SOL / 10,
        ),
        "fund stranger",
    );

    let upload = sign_tx(
        &[&stranger],
        &[ix(
            instruction::UploadConfig { rewards: rewards() },
            accounts::UploadConfig {
                authority: as_pubkey(&stranger.pubkey()),
                machine: setup.machine,
            },
        )],
        setup.svm.latest_blockhash_for(TransactionTarget::Base),
    );
    let failed = setup
        .svm
        .send_transaction_to(TransactionTarget::Base, upload)
        .expect_err("a non-authority upload must fail");
    assert_eq!(
        failed.err,
        TransactionError::InstructionError(0, InstructionError::Custom(CONSTRAINT_HAS_ONE))
    );
    let machine: Machine = fetch(&setup.svm, setup.machine);
    assert_eq!(machine.total_weight, 0);
}

#[test]
fn pull_records_the_pending_pull_and_requests_randomness() {
    let mut setup = init_machine();
    upload_config(&mut setup);
    let player = as_pubkey(&setup.authority.pubkey());
    let pull_id = 1u64;
    let pending_pull = pda(&[
        PULL_SEED,
        setup.machine.as_ref(),
        player.as_ref(),
        &pull_id.to_le_bytes(),
    ]);
    let asset = pda(&[
        ASSET_SEED,
        setup.machine.as_ref(),
        player.as_ref(),
        &pull_id.to_le_bytes(),
    ]);
    let queue_before = balance(&setup.svm, DEFAULT_VRF_QUEUE);

    send_ix(
        &mut setup.svm,
        TransactionTarget::Base,
        &[&setup.authority],
        ix(
            instruction::Pull {
                pull_id,
                client_seed: 9,
            },
            accounts::Pull {
                player,
                machine: setup.machine,
                pending_pull,
                asset,
                treasury: setup.treasury,
                update_authority: setup.update_authority,
                callback_identity: pda(&[VRF_IDENTITY_SEED]),
                oracle_queue: DEFAULT_VRF_QUEUE,
                vrf_program: VRF_PROGRAM_ID,
                slot_hashes: SLOT_HASHES_ID,
                system_program: system_program::ID,
                mpl_core_program: MPL_CORE_ID,
            },
        ),
        "pull",
    );

    let pull: PendingPull = fetch(&setup.svm, pending_pull);
    assert_eq!(pull.machine, setup.machine);
    assert_eq!(pull.player, player);
    assert_eq!(pull.asset, asset);
    assert_eq!(pull.pull_id, pull_id);
    assert_eq!(pull.reward_id, u8::MAX);
    assert_eq!(pull.status, 0, "pending until the VRF callback");
    assert_eq!(
        balance(&setup.svm, setup.treasury),
        2 * TREASURY_TOP_UP_LAMPORTS,
        "the pull fee tops up the treasury"
    );
    assert_eq!(
        balance(&setup.svm, DEFAULT_VRF_QUEUE),
        queue_before + VRF_HIGH_PRIORITY_LAMPORTS_COST,
        "the VRF program took the request"
    );
}
