//! The Pinocchio programs read their id from the runtime, so each test loads
//! them at a fresh address.

use {
    magicsvm::{MagicSVM, TransactionTarget},
    solana_address::Address,
    solana_clock::Clock,
    solana_instruction::{AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_native_token::LAMPORTS_PER_SOL,
    solana_signer::Signer,
    test_utils::{add_program, airdrop, program_so_path, send_ix},
};

const VRF_PROGRAM_ID: Address =
    Address::from_str_const("Vrf1RNUjXmQGjmQrQLvJHs9SNkvDJEsRVFPkfSQUwGz");
const DEFAULT_QUEUE: Address =
    Address::from_str_const("Cuj97ggrhhidhbu39TijNVqE74xvKJ69gDervRUXAxGh");
const DEFAULT_EPHEMERAL_QUEUE: Address =
    Address::from_str_const("5hBR571xnXppuCPveTrctfTU7tJLSN94nq7kv7FRK5Tc");
const DELEGATION_PROGRAM_ID: Address =
    Address::from_str_const("DELeGGvXpWV2fqJUhqcF5ZSYMS4JTLjteaAMARRSaeSh");
const MAGIC_PROGRAM_ID: Address =
    Address::from_str_const("Magic11111111111111111111111111111111111111");
const MAGIC_CONTEXT_ID: Address =
    Address::from_str_const("MagicContext1111111111111111111111111111111");
const SYSTEM_PROGRAM_ID: Address = Address::from_str_const("11111111111111111111111111111111");
const SLOT_HASHES: Address = Address::from_str_const("SysvarS1otHashes111111111111111111111111111");
/// The programs request high-priority randomness.
const VRF_HIGH_PRIORITY_LAMPORTS_COST: u64 = 800_000;

const INITIALIZE: u64 = 0;
const ROLL_DICE: u64 = 1;
const DELEGATE_PLAYER: u64 = 3;
const UNDELEGATE_PLAYER: u64 = 4;

struct Dice {
    svm: MagicSVM,
    program: Address,
    payer: Keypair,
    player: Address,
}

fn setup(program_name: &str) -> Dice {
    let mut svm = MagicSVM::new();
    let program = Address::new_unique();
    add_program(
        &mut svm,
        program,
        program_so_path(env!("CARGO_MANIFEST_DIR"), program_name),
    );
    let payer = Keypair::new();
    airdrop(&mut svm, &payer.pubkey(), 2 * LAMPORTS_PER_SOL);
    let player = Address::find_program_address(&[b"player", payer.pubkey().as_ref()], &program).0;
    let mut dice = Dice {
        svm,
        program,
        payer,
        player,
    };
    let accounts = vec![
        AccountMeta::new(dice.payer.pubkey(), true),
        AccountMeta::new(player, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];
    dice.send(
        TransactionTarget::Base,
        INITIALIZE,
        &[],
        accounts,
        "initialize",
    );
    dice
}

impl Dice {
    fn send(
        &mut self,
        target: TransactionTarget,
        discriminator: u64,
        payload: &[u8],
        accounts: Vec<AccountMeta>,
        label: &str,
    ) {
        let instruction = Instruction {
            program_id: self.program,
            accounts,
            data: [&discriminator.to_le_bytes()[..], payload].concat(),
        };
        send_ix(&mut self.svm, target, &[&self.payer], instruction, label);
    }

    fn roll(&mut self, target: TransactionTarget, queue: Address, client_seed: u8) {
        let identity = Address::find_program_address(&[b"identity"], &self.program).0;
        let accounts = vec![
            AccountMeta::new(self.payer.pubkey(), true),
            AccountMeta::new_readonly(self.player, false),
            AccountMeta::new(queue, false),
            AccountMeta::new_readonly(identity, false),
            AccountMeta::new_readonly(VRF_PROGRAM_ID, false),
            AccountMeta::new_readonly(SLOT_HASHES, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ];
        self.send(target, ROLL_DICE, &[client_seed], accounts, "roll_dice");
    }

    /// Advances one slot, fulfilling the pending request.
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

    /// `(last_result, rollnum)`.
    fn state(&self, target: TransactionTarget) -> (u8, u8) {
        let data = self
            .svm
            .get_account_for(target, &self.player)
            .expect("player account")
            .data;
        (data[0], data[1])
    }
}

#[test]
fn base_roll_pays_the_high_priority_fee_and_gets_its_callback() {
    let mut dice = setup("roll_dice");
    for roll in 1..=3u8 {
        let queue_before = dice.svm.get_balance(&DEFAULT_QUEUE).unwrap();
        dice.roll(TransactionTarget::Base, DEFAULT_QUEUE, roll);
        assert_eq!(dice.state(TransactionTarget::Base).1, roll - 1, "pending");
        assert_eq!(
            dice.svm.get_balance(&DEFAULT_QUEUE).unwrap(),
            queue_before + VRF_HIGH_PRIORITY_LAMPORTS_COST
        );

        dice.fulfill();
        let (last_result, rollnum) = dice.state(TransactionTarget::Base);
        assert_eq!(rollnum, roll);
        assert!((1..=6).contains(&last_result), "{last_result}");
    }
}

#[test]
fn delegated_roll_gets_its_callback_on_the_er_and_commits() {
    let mut dice = setup("roll_dice_delegated");
    let (program, player) = (dice.program, dice.player);
    let delegation_pda = |seed: &[u8]| {
        Address::find_program_address(&[seed, player.as_ref()], &DELEGATION_PROGRAM_ID).0
    };
    let accounts = vec![
        AccountMeta::new(dice.payer.pubkey(), true),
        AccountMeta::new(player, false),
        AccountMeta::new_readonly(program, false),
        AccountMeta::new(
            Address::find_program_address(&[b"buffer", player.as_ref()], &program).0,
            false,
        ),
        AccountMeta::new(delegation_pda(b"delegation"), false),
        AccountMeta::new(delegation_pda(b"delegation-metadata"), false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(DELEGATION_PROGRAM_ID, false),
        AccountMeta::new_readonly(dice.svm.validator_identity(), false),
    ];
    dice.send(
        TransactionTarget::Base,
        DELEGATE_PLAYER,
        &[],
        accounts,
        "delegate",
    );
    let owner = |dice: &Dice| dice.svm.get_account(&player).unwrap().owner;
    assert_eq!(owner(&dice), DELEGATION_PROGRAM_ID);

    for roll in 1..=2u8 {
        dice.roll(TransactionTarget::Ephemeral, DEFAULT_EPHEMERAL_QUEUE, roll);
        dice.fulfill();
        assert_eq!(dice.state(TransactionTarget::Ephemeral).1, roll);
    }
    let rolled = dice.state(TransactionTarget::Ephemeral);

    let accounts = vec![
        AccountMeta::new(dice.payer.pubkey(), true),
        AccountMeta::new(player, false),
        AccountMeta::new(MAGIC_CONTEXT_ID, false),
        AccountMeta::new_readonly(MAGIC_PROGRAM_ID, false),
    ];
    dice.send(
        TransactionTarget::Ephemeral,
        UNDELEGATE_PLAYER,
        &[],
        accounts,
        "undelegate",
    );
    assert_eq!(owner(&dice), program);
    assert_eq!(dice.state(TransactionTarget::Base), rolled);
}
