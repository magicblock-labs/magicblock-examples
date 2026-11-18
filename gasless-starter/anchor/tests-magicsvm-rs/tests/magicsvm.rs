//! The tip-jar flow from `scripts/create-jar.ts` and the app, in-process.
//!
//! The hosted Payments API isn't available here, so the test plays the sponsor:
//! `sponsor` pays every base-layer fee. The tipper and the jar authority never hold
//! SOL, and every ER transaction uses one of them as the fee payer.

use {
    anchor_lang::{prelude::Pubkey, system_program, AccountDeserialize, Id},
    ephemeral_rollups_sdk::{
        consts::{ASSOCIATED_TOKEN_PROGRAM_ID, TOKEN_PROGRAM_ID},
        spl::{
            builders::{
                DelegateEphemeralAtaBuilder, DepositSplTokensBuilder,
                InitializeEphemeralAtaBuilder, InitializeGlobalVaultBuilder,
            },
            find_rent_pda, GlobalVault,
        },
    },
    magicsvm::{MagicSVM, TransactionTarget},
    solana_address::Address,
    solana_instruction::{error::InstructionError, AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_native_token::LAMPORTS_PER_SOL,
    solana_signer::Signer,
    solana_system_interface::instruction as system_instruction,
    solana_transaction_error::TransactionError,
    test_utils::{
        as_address, as_pubkey, boot, client_ix,
        delegate_buffer_pda_from_delegated_account_and_owner_program,
        delegation_metadata_pda_from_delegated_account,
        delegation_record_pda_from_delegated_account, program_id_from_idl, send_ixs, sign_tx,
        DelegationProgram, MagicProgram, MAGIC_CONTEXT_ID,
    },
    tip_jar::{accounts, instruction, Jar, JAR_SEED},
};

const MINT_SIZE: usize = 82;
const DECIMALS: u8 = 6;
const MINTED: u64 = 10_000_000;
const DEPOSIT: u64 = 5_000_000;
const TIP: u64 = 2_000_000;

fn program_id() -> Pubkey {
    as_pubkey(&program_id_from_idl(env!("CARGO_MANIFEST_DIR"), "tip_jar.json"))
}

fn ix(
    data: impl anchor_lang::InstructionData,
    accounts: impl anchor_lang::ToAccountMetas,
) -> Instruction {
    client_ix(as_address(program_id()), data, accounts)
}

fn ata(owner: &Address, mint: &Address) -> Address {
    Address::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

fn token_balance(svm: &MagicSVM, target: TransactionTarget, account: &Address) -> u64 {
    let account = svm
        .get_account_for(target, account)
        .unwrap_or_else(|| panic!("missing token account {account} on {target:?}"));
    u64::from_le_bytes(account.data[64..72].try_into().unwrap())
}

fn lamports(svm: &MagicSVM, target: TransactionTarget, address: &Address) -> u64 {
    svm.get_account_for(target, address)
        .map_or(0, |account| account.lamports)
}

fn jar_state(svm: &MagicSVM, target: TransactionTarget, jar: &Address) -> Jar {
    let account = svm
        .get_account_for(target, jar)
        .unwrap_or_else(|| panic!("missing jar on {target:?}"));
    Jar::try_deserialize(&mut &account.data[..]).unwrap()
}

fn create_ata_ix(payer: Address, owner: Address, mint: Address) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(ata(&owner, &mint), false),
            AccountMeta::new_readonly(owner, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(solana_sdk_ids::system_program::id(), false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        // CreateIdempotent
        data: vec![1],
    }
}

fn initialize_mint_ix(mint: Address, authority: Address) -> Instruction {
    let mut data = vec![0u8, DECIMALS];
    data.extend_from_slice(authority.as_ref());
    data.push(0); // no freeze authority
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(mint, false),
            AccountMeta::new_readonly(solana_sdk_ids::sysvar::rent::id(), false),
        ],
        data,
    }
}

fn mint_to_ix(mint: Address, destination: Address, authority: Address, amount: u64) -> Instruction {
    let mut data = vec![7u8];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(mint, false),
            AccountMeta::new(destination, false),
            AccountMeta::new_readonly(authority, true),
        ],
        data,
    }
}

/// Delegate `owner`'s ephemeral ATA so the ER can write its token account. Works for
/// wallets and PDAs alike; only the payer signs.
fn custody_ixs(payer: Address, owner: Address, mint: Address, validator: Address) -> Vec<Instruction> {
    vec![
        create_ata_ix(payer, owner, mint),
        InitializeEphemeralAtaBuilder {
            payer,
            user: owner,
            mint,
        }
        .instruction(),
        DelegateEphemeralAtaBuilder {
            payer,
            user: owner,
            mint,
            validator: Some(validator),
        }
        .instruction(),
    ]
}

/// The instruction set the Payments API's `/v1/spl/deposit` returns, minus its relay
/// fee: set up the mint's vault, move `amount` into the owner's ephemeral ATA, and
/// delegate it. The owner signs only as token owner.
fn deposit_ixs(
    payer: Address,
    owner: Address,
    mint: Address,
    amount: u64,
    validator: Address,
) -> Vec<Instruction> {
    let (vault, _) = GlobalVault::find_pda(&mint);
    vec![
        InitializeEphemeralAtaBuilder {
            payer,
            user: owner,
            mint,
        }
        .instruction(),
        InitializeGlobalVaultBuilder { payer, mint }.instruction(),
        // The vault's token account is its ATA.
        create_ata_ix(payer, vault, mint),
        DelegateEphemeralAtaBuilder {
            payer,
            user: vault,
            mint,
            validator: Some(validator),
        }
        .instruction(),
        DepositSplTokensBuilder {
            authority: owner,
            user: owner,
            mint,
            amount,
        }
        .instruction(),
        DelegateEphemeralAtaBuilder {
            payer,
            user: owner,
            mint,
            validator: Some(validator),
        }
        .instruction(),
    ]
}

fn tip_ix(tipper: Address, jar: Address, mint: Address, amount: u64) -> Instruction {
    ix(
        instruction::Tip { amount },
        accounts::Tip {
            tipper: as_pubkey(&tipper),
            tipper_token: as_pubkey(&ata(&tipper, &mint)),
            jar: as_pubkey(&jar),
            jar_token: as_pubkey(&ata(&jar, &mint)),
            token_program: as_pubkey(&TOKEN_PROGRAM_ID),
        },
    )
}

fn withdraw_ix(
    authority: Address,
    jar: Address,
    destination: Address,
    mint: Address,
    amount: u64,
) -> Instruction {
    ix(
        instruction::WithdrawTips { amount },
        accounts::WithdrawTips {
            authority: as_pubkey(&authority),
            jar: as_pubkey(&jar),
            jar_token: as_pubkey(&ata(&jar, &mint)),
            destination: as_pubkey(&destination),
            token_program: as_pubkey(&TOKEN_PROGRAM_ID),
        },
    )
}

#[test]
fn tips_and_withdrawals_run_on_the_er_from_wallets_with_no_sol() {
    let sponsor = Keypair::new();
    let mut svm = boot(
        env!("CARGO_MANIFEST_DIR"),
        "tip_jar",
        as_address(program_id()),
        &sponsor,
        10 * LAMPORTS_PER_SOL,
    );
    let validator = svm.validator_identity();
    let mint = Keypair::new();
    let tipper = Keypair::new();
    let authority = Keypair::new();
    let jar = as_address(
        Pubkey::find_program_address(&[JAR_SEED, authority.pubkey().as_ref()], &program_id()).0,
    );
    let jar_pubkey = as_pubkey(&jar);

    // Fund the ESPL rent PDA (pays for deposit shuttles) and create a 6-decimal mint
    // with 10 tokens for the tipper.
    let mint_rent = svm.minimum_balance_for_rent_exemption(MINT_SIZE);
    send_ixs(
        &mut svm,
        TransactionTarget::Base,
        &[&sponsor, &mint],
        [
            system_instruction::transfer(&sponsor.pubkey(), &find_rent_pda().0, LAMPORTS_PER_SOL / 5),
            system_instruction::create_account(
                &sponsor.pubkey(),
                &mint.pubkey(),
                mint_rent,
                MINT_SIZE as u64,
                &TOKEN_PROGRAM_ID,
            ),
            initialize_mint_ix(mint.pubkey(), sponsor.pubkey()),
            create_ata_ix(sponsor.pubkey(), tipper.pubkey(), mint.pubkey()),
            mint_to_ix(
                mint.pubkey(),
                ata(&tipper.pubkey(), &mint.pubkey()),
                sponsor.pubkey(),
                MINTED,
            ),
        ],
        "create mint",
    );

    // One-time setup on the base layer, paid by the sponsor: create the jar, delegate
    // the jar's token account and the jar itself, and give the authority a token
    // account on the ER to withdraw into.
    let create_jar = ix(
        instruction::CreateJar {
            validator: as_pubkey(&validator),
        },
        accounts::CreateJar {
            payer: as_pubkey(&sponsor.pubkey()),
            authority: as_pubkey(&authority.pubkey()),
            jar: jar_pubkey,
            mint: as_pubkey(&mint.pubkey()),
            jar_token: as_pubkey(&ata(&jar, &mint.pubkey())),
            token_program: as_pubkey(&TOKEN_PROGRAM_ID),
            associated_token_program: as_pubkey(&ASSOCIATED_TOKEN_PROGRAM_ID),
            system_program: system_program::ID,
        },
    );
    let delegate_jar = ix(
        instruction::DelegateJar {},
        accounts::DelegateJar {
            payer: as_pubkey(&sponsor.pubkey()),
            buffer_jar: delegate_buffer_pda_from_delegated_account_and_owner_program(
                &jar_pubkey,
                &program_id(),
            ),
            delegation_record_jar: delegation_record_pda_from_delegated_account(&jar_pubkey),
            delegation_metadata_jar: delegation_metadata_pda_from_delegated_account(&jar_pubkey),
            jar: jar_pubkey,
            owner_program: program_id(),
            delegation_program: DelegationProgram::id(),
            system_program: system_program::ID,
        },
    );
    let mut setup = vec![create_jar];
    setup.extend(custody_ixs(sponsor.pubkey(), jar, mint.pubkey(), validator));
    setup.push(delegate_jar);
    setup.extend(custody_ixs(
        sponsor.pubkey(),
        authority.pubkey(),
        mint.pubkey(),
        validator,
    ));
    send_ixs(
        &mut svm,
        TransactionTarget::Base,
        &[&sponsor, &authority],
        setup,
        "create and delegate the jar",
    );
    let saved = jar_state(&svm, TransactionTarget::Base, &jar);
    assert_eq!(saved.authority, as_pubkey(&authority.pubkey()));
    assert_eq!(saved.validator, as_pubkey(&validator));
    assert_eq!(token_balance(&svm, TransactionTarget::Ephemeral, &ata(&jar, &mint.pubkey())), 0);

    // Deposit the tipper's tokens into the ER.
    send_ixs(
        &mut svm,
        TransactionTarget::Base,
        &[&sponsor, &tipper],
        deposit_ixs(
            sponsor.pubkey(),
            tipper.pubkey(),
            mint.pubkey(),
            DEPOSIT,
            validator,
        ),
        "deposit",
    );
    let tipper_token = ata(&tipper.pubkey(), &mint.pubkey());
    assert_eq!(token_balance(&svm, TransactionTarget::Ephemeral, &tipper_token), DEPOSIT);

    // Tip on the ER from a wallet with zero SOL.
    send_ixs(
        &mut svm,
        TransactionTarget::Ephemeral,
        &[&tipper],
        [tip_ix(tipper.pubkey(), jar, mint.pubkey(), TIP)],
        "tip",
    );
    let jar_token = ata(&jar, &mint.pubkey());
    assert_eq!(
        token_balance(&svm, TransactionTarget::Ephemeral, &tipper_token),
        DEPOSIT - TIP
    );
    assert_eq!(token_balance(&svm, TransactionTarget::Ephemeral, &jar_token), TIP);
    assert_eq!(lamports(&svm, TransactionTarget::Base, &tipper.pubkey()), 0);

    // Anyone but the authority is rejected by the jar's `has_one = authority`.
    let steal = sign_tx(
        &[&tipper],
        &[withdraw_ix(tipper.pubkey(), jar, tipper_token, mint.pubkey(), 1)],
        svm.latest_blockhash_for(TransactionTarget::Ephemeral),
    );
    let failed = svm
        .send_transaction_to(TransactionTarget::Ephemeral, steal)
        .expect_err("a non-authority withdrawal must fail");
    svm.expire_blockhash_for(TransactionTarget::Ephemeral);
    assert_eq!(
        failed.err,
        TransactionError::InstructionError(0, InstructionError::Custom(2001))
    );
    assert_eq!(token_balance(&svm, TransactionTarget::Ephemeral, &jar_token), TIP);

    // The authority collects the tips on the ER; the jar PDA signs the transfer.
    let authority_token = ata(&authority.pubkey(), &mint.pubkey());
    send_ixs(
        &mut svm,
        TransactionTarget::Ephemeral,
        &[&authority],
        [withdraw_ix(
            authority.pubkey(),
            jar,
            authority_token,
            mint.pubkey(),
            TIP,
        )],
        "withdraw",
    );
    assert_eq!(token_balance(&svm, TransactionTarget::Ephemeral, &jar_token), 0);
    assert_eq!(
        token_balance(&svm, TransactionTarget::Ephemeral, &authority_token),
        TIP
    );

    // Commit the jar's totals back to the base layer. It stays delegated.
    send_ixs(
        &mut svm,
        TransactionTarget::Ephemeral,
        &[&authority],
        [ix(
            instruction::CommitJar {},
            accounts::CommitJar {
                authority: as_pubkey(&authority.pubkey()),
                jar: jar_pubkey,
                magic_program: MagicProgram::id(),
                magic_context: MAGIC_CONTEXT_ID,
            },
        )],
        "commit",
    );
    let committed = jar_state(&svm, TransactionTarget::Base, &jar);
    assert_eq!(committed.tip_count, 1);
    assert_eq!(committed.total_tipped, TIP);
    assert_eq!(lamports(&svm, TransactionTarget::Base, &authority.pubkey()), 0);
}
