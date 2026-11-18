use {
    anchor_lang::{prelude::Pubkey, system_program, Id},
    crank_counter::{accounts, instruction, ScheduleIncrementArgs, COUNTER_SEED},
    magicsvm::{MagicSVM, TransactionTarget},
    solana_address::Address,
    solana_clock::Clock,
    solana_instruction::{AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_native_token::LAMPORTS_PER_SOL,
    solana_signer::Signer,
    test_utils::{
        account_u64, as_address, as_pubkey, boot, client_ix,
        delegate_buffer_pda_from_delegated_account_and_owner_program,
        delegation_metadata_pda_from_delegated_account,
        delegation_record_pda_from_delegated_account, is_delegated, program_id_from_idl, send_ix,
        DelegationProgram, MagicProgram, MAGIC_CONTEXT_ID,
    },
};

fn program_id() -> Pubkey {
    as_pubkey(&program_id_from_idl(
        env!("CARGO_MANIFEST_DIR"),
        "anchor_counter.json",
    ))
}

fn ix(
    data: impl anchor_lang::InstructionData,
    accounts: impl anchor_lang::ToAccountMetas,
) -> Instruction {
    client_ix(as_address(program_id()), data, accounts)
}

fn count(svm: &MagicSVM, target: TransactionTarget, counter: &Address) -> u64 {
    account_u64(svm, target, counter, 8)
}

/// Warps to `slot` and asserts every crank run on the way succeeded.
fn warp(svm: &mut MagicSVM, slot: u64) -> usize {
    let results = svm.warp_to_slot(slot);
    for result in &results {
        if let Err(failed) = result {
            panic!(
                "crank failed: {:?}\nlogs: {:#?}",
                failed.err, failed.meta.logs
            );
        }
    }
    results.len()
}

#[test]
fn scheduled_increment_runs_on_the_er_and_commits() {
    let payer = Keypair::new();
    let mut svm = boot(
        env!("CARGO_MANIFEST_DIR"),
        "crank_counter",
        as_address(program_id()),
        &payer,
        2 * LAMPORTS_PER_SOL,
    );
    let user = as_pubkey(&payer.pubkey());
    let (counter, _) = Pubkey::find_program_address(&[COUNTER_SEED], &program_id());
    let counter_address = as_address(counter);

    send_ix(
        &mut svm,
        TransactionTarget::Base,
        &[&payer],
        ix(
            instruction::Initialize {},
            accounts::Initialize {
                counter,
                user,
                system_program: system_program::ID,
            },
        ),
        "initialize",
    );
    let mut delegate = ix(
        instruction::Delegate {},
        accounts::DelegateInput {
            payer: user,
            buffer_pda: delegate_buffer_pda_from_delegated_account_and_owner_program(
                &counter,
                &program_id(),
            ),
            delegation_record_pda: delegation_record_pda_from_delegated_account(&counter),
            delegation_metadata_pda: delegation_metadata_pda_from_delegated_account(&counter),
            pda: counter,
            owner_program: program_id(),
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
        &[&payer],
        delegate,
        "delegate",
    );
    assert!(is_delegated(&svm, &counter_address));

    // 5 increments, 100 ms apart: two 50 ms ER slots between runs.
    send_ix(
        &mut svm,
        TransactionTarget::Ephemeral,
        &[&payer],
        ix(
            instruction::ScheduleIncrement {
                args: ScheduleIncrementArgs {
                    task_id: 1,
                    execution_interval_millis: 100,
                    iterations: 5,
                },
            },
            accounts::ScheduleIncrement {
                magic_program: MagicProgram::id(),
                payer: user,
                counter,
                program: program_id(),
            },
        ),
        "schedule_increment",
    );
    let scheduled_at = svm.ephemeral().get_sysvar::<Clock>().slot;
    assert_eq!(
        count(&svm, TransactionTarget::Ephemeral, &counter_address),
        0
    );

    assert_eq!(
        warp(&mut svm, scheduled_at),
        0,
        "not in the scheduling slot"
    );
    assert_eq!(warp(&mut svm, scheduled_at + 1), 1);
    assert_eq!(
        count(&svm, TransactionTarget::Ephemeral, &counter_address),
        1
    );
    assert_eq!(warp(&mut svm, scheduled_at + 2), 0);
    assert_eq!(warp(&mut svm, scheduled_at + 3), 1);
    assert_eq!(
        count(&svm, TransactionTarget::Ephemeral, &counter_address),
        2
    );
    assert_eq!(
        warp(&mut svm, scheduled_at + 100),
        3,
        "the rest, then the task ends"
    );
    assert_eq!(
        count(&svm, TransactionTarget::Ephemeral, &counter_address),
        5
    );
    assert_eq!(warp(&mut svm, scheduled_at + 1_000), 0);

    send_ix(
        &mut svm,
        TransactionTarget::Ephemeral,
        &[&payer],
        ix(
            instruction::Undelegate {},
            accounts::UndelegateInput {
                payer: user,
                counter,
                magic_program: MagicProgram::id(),
                magic_context: MAGIC_CONTEXT_ID,
            },
        ),
        "undelegate",
    );
    assert!(!is_delegated(&svm, &counter_address));
    assert_eq!(count(&svm, TransactionTarget::Base, &counter_address), 5);
}
