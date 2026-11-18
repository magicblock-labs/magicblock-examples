use {
    anchor_lang::{prelude::Pubkey, system_program, Id},
    magicsvm::{MagicSVM, TransactionTarget},
    roll_dice_delegated::PLAYER_SEED,
    solana_clock::Clock,
    solana_instruction::{AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_native_token::LAMPORTS_PER_SOL,
    solana_signer::Signer,
    test_utils::{
        as_address, as_pubkey, boot, client_ix,
        delegate_buffer_pda_from_delegated_account_and_owner_program,
        delegation_metadata_pda_from_delegated_account,
        delegation_record_pda_from_delegated_account, is_delegated, program_id_from_idl, send_ix,
        DelegationProgram, MagicProgram, MAGIC_CONTEXT_ID,
    },
};

const VRF_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz");
const DEFAULT_QUEUE: Pubkey =
    Pubkey::from_str_const("Cuj97ggrhhidhbu39TijNVqE74xvKJ69gDervRUXAxGh");
const DEFAULT_EPHEMERAL_QUEUE: Pubkey =
    Pubkey::from_str_const("5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc");
const SLOT_HASHES: Pubkey = Pubkey::from_str_const("SysvarS1otHashes111111111111111111111111111");
const VRF_LAMPORTS_COST: u64 = 500_000;

fn program_id(idl: &str) -> Pubkey {
    as_pubkey(&program_id_from_idl(env!("CARGO_MANIFEST_DIR"), idl))
}

fn ix(
    program: Pubkey,
    data: impl anchor_lang::InstructionData,
    accounts: impl anchor_lang::ToAccountMetas,
) -> Instruction {
    client_ix(as_address(program), data, accounts)
}

/// `(last_result, rollnum)` of a player account.
fn player_state(svm: &MagicSVM, target: TransactionTarget, player: Pubkey) -> (u8, u8) {
    let data = svm
        .get_account_for(target, &as_address(player))
        .expect("player account")
        .data;
    (data[8], data[9])
}

/// Advances one slot, which fulfills pending randomness requests, and asserts
/// exactly `expected` callbacks ran and succeeded.
fn fulfill(svm: &mut MagicSVM, expected: usize) {
    let slot = svm.get_sysvar::<Clock>().slot;
    let results = svm.warp_to_slot(slot + 1);
    assert_eq!(results.len(), expected);
    for result in results {
        if let Err(failed) = result {
            panic!(
                "callback failed: {:?}\nlogs: {:#?}",
                failed.err, failed.meta.logs
            );
        }
    }
}

#[test]
fn base_roll_gets_its_callback_in_the_next_slot() {
    let payer = Keypair::new();
    let program = program_id("random_dice.json");
    let mut svm = boot(
        env!("CARGO_MANIFEST_DIR"),
        "roll_dice",
        as_address(program),
        &payer,
        2 * LAMPORTS_PER_SOL,
    );
    let user = as_pubkey(&payer.pubkey());
    let (player, _) = Pubkey::find_program_address(&[roll_dice::PLAYER, user.as_ref()], &program);
    let (program_identity, _) = Pubkey::find_program_address(&[b"identity"], &program);

    send_ix(
        &mut svm,
        TransactionTarget::Base,
        &[&payer],
        ix(
            program,
            roll_dice::instruction::Initialize {},
            roll_dice::accounts::Initialize {
                payer: user,
                player,
                system_program: system_program::ID,
            },
        ),
        "initialize",
    );

    for roll in 1..=3u8 {
        let queue_before = svm.get_balance(&as_address(DEFAULT_QUEUE)).unwrap();
        send_ix(
            &mut svm,
            TransactionTarget::Base,
            &[&payer],
            ix(
                program,
                roll_dice::instruction::RollDice { client_seed: roll },
                roll_dice::accounts::DoRollDiceCtx {
                    payer: user,
                    player,
                    oracle_queue: DEFAULT_QUEUE,
                    program_identity,
                    vrf_program: VRF_PROGRAM_ID,
                    slot_hashes: SLOT_HASHES,
                    system_program: system_program::ID,
                },
            ),
            "roll_dice",
        );
        assert_eq!(
            player_state(&svm, TransactionTarget::Base, player).1,
            roll - 1,
            "pending until the next slot"
        );
        assert_eq!(
            svm.get_balance(&as_address(DEFAULT_QUEUE)).unwrap(),
            queue_before + VRF_LAMPORTS_COST
        );

        fulfill(&mut svm, 1);
        let (last_result, rollnum) = player_state(&svm, TransactionTarget::Base, player);
        assert_eq!(rollnum, roll);
        assert!((1..=6).contains(&last_result), "{last_result}");
    }
}

#[test]
fn delegated_roll_gets_its_callback_on_the_er_and_commits() {
    let payer = Keypair::new();
    let program = program_id("random_dice_delegated.json");
    let mut svm = boot(
        env!("CARGO_MANIFEST_DIR"),
        "roll_dice_delegated",
        as_address(program),
        &payer,
        2 * LAMPORTS_PER_SOL,
    );
    let user = as_pubkey(&payer.pubkey());
    let (player, _) = Pubkey::find_program_address(&[PLAYER_SEED, user.as_ref()], &program);
    let (program_identity, _) = Pubkey::find_program_address(&[b"identity"], &program);

    send_ix(
        &mut svm,
        TransactionTarget::Base,
        &[&payer],
        ix(
            program,
            roll_dice_delegated::instruction::Initialize {},
            roll_dice_delegated::accounts::Initialize {
                payer: user,
                player,
                system_program: system_program::ID,
            },
        ),
        "initialize",
    );
    let mut delegate = ix(
        program,
        roll_dice_delegated::instruction::Delegate {},
        roll_dice_delegated::accounts::DelegateInput {
            user,
            buffer_player: delegate_buffer_pda_from_delegated_account_and_owner_program(
                &player, &program,
            ),
            delegation_record_player: delegation_record_pda_from_delegated_account(&player),
            delegation_metadata_player: delegation_metadata_pda_from_delegated_account(&player),
            player,
            owner_program: program,
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
    assert!(is_delegated(&svm, &as_address(player)));

    for roll in 1..=2u8 {
        send_ix(
            &mut svm,
            TransactionTarget::Ephemeral,
            &[&payer],
            ix(
                program,
                roll_dice_delegated::instruction::RollDiceDelegated { client_seed: roll },
                roll_dice_delegated::accounts::DoRollDiceDelegatedCtx {
                    payer: user,
                    player,
                    oracle_queue: DEFAULT_EPHEMERAL_QUEUE,
                    program_identity,
                    vrf_program: VRF_PROGRAM_ID,
                    slot_hashes: SLOT_HASHES,
                    system_program: system_program::ID,
                },
            ),
            "roll_dice_delegated",
        );
        fulfill(&mut svm, 1);
        assert_eq!(
            player_state(&svm, TransactionTarget::Ephemeral, player).1,
            roll
        );
    }
    let rolled = player_state(&svm, TransactionTarget::Ephemeral, player);

    send_ix(
        &mut svm,
        TransactionTarget::Ephemeral,
        &[&payer],
        ix(
            program,
            roll_dice_delegated::instruction::Undelegate {},
            roll_dice_delegated::accounts::Undelegate {
                payer: user,
                user: player,
                magic_program: MagicProgram::id(),
                magic_context: MAGIC_CONTEXT_ID,
            },
        ),
        "undelegate",
    );
    assert!(!is_delegated(&svm, &as_address(player)));
    assert_eq!(player_state(&svm, TransactionTarget::Base, player), rolled);
}
