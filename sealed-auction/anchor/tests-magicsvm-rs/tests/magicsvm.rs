//! The sealed-bid auction, in-process: the Token A lot stays in L1 custody, bids
//! are private ephemeral accounts on the ER that only the auctioneer and the bidder
//! can read, and the auction settles, refunds and undelegates before the winner
//! claims the lot on L1.

use {
    anchor_lang::{
        prelude::Pubkey, system_program, AnchorDeserialize, Id, InstructionData, ToAccountMetas,
    },
    ephemeral_rollups_sdk::{
        access_control::structs::EphemeralPermission,
        consts::{
            ASSOCIATED_TOKEN_PROGRAM_ID, DELEGATION_PROGRAM_ID, EPHEMERAL_VAULT_ID,
            ESPL_TOKEN_PROGRAM_ID, MAGIC_PROGRAM_ID, PERMISSION_PROGRAM_ID, TOKEN_PROGRAM_ID,
        },
        spl::{
            builders::{
                DelegateEphemeralAtaBuilder, DepositSplTokensBuilder,
                InitializeEphemeralAtaBuilder, InitializeGlobalVaultBuilder,
            },
            find_rent_pda, GlobalVault,
        },
    },
    magicsvm::{MagicSVM, TransactionTarget},
    sealed_auction::{accounts, instruction, AUCTION_SEED, BID_SEED},
    solana_address::Address,
    solana_clock::Clock,
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
};

const MINT_SIZE: usize = 82;
const LOT_AMOUNT: u64 = 100;
const BID_ONE: u64 = 40;
const BID_TWO: u64 = 55;
const SPONSOR_LAMPORTS: u64 = 100_000_000;
const START_TS: i64 = 1_800_000_000;

// The program's `ErrorCode`, offset by Anchor's 6000.
const INVALID_AMOUNT: u32 = 6000;
const DEADLINE_IN_PAST: u32 = 6001;
const AUCTION_STILL_OPEN: u32 = 6002;
const AUCTION_NOT_ENDED: u32 = 6004;
const WINNER_CANNOT_REFUND: u32 = 6008;

/// Mirror of the program's `Auction` account, which the crate keeps private.
#[derive(AnchorDeserialize)]
struct Auction {
    auctioneer: Pubkey,
    auction_id: u64,
    token_a_mint: Pubkey,
    token_b_mint: Pubkey,
    lot_amount: u64,
    deadline_ts: i64,
    bid_count: u8,
    closed_bid_count: u8,
    highest_bid: u64,
    highest_bidder: Pubkey,
    status: AuctionStatus,
    lot_claimed: bool,
    _bump: u8,
}

#[derive(AnchorDeserialize, Debug, PartialEq)]
enum AuctionStatus {
    Open,
    Ended,
    Settled,
}

fn program_id() -> Pubkey {
    as_pubkey(&program_id_from_idl(
        env!("CARGO_MANIFEST_DIR"),
        "sealed_auction.json",
    ))
}

fn ix(data: impl InstructionData, accounts: impl ToAccountMetas) -> Instruction {
    client_ix(as_address(program_id()), data, accounts)
}

fn pk(address: &Address) -> Pubkey {
    as_pubkey(address)
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

fn auction_state(svm: &MagicSVM, target: TransactionTarget, auction: &Address) -> Auction {
    let account = svm
        .get_account_for(target, auction)
        .unwrap_or_else(|| panic!("missing auction on {target:?}"));
    Auction::deserialize(&mut &account.data[8..]).unwrap()
}

fn set_time(svm: &mut MagicSVM, unix_timestamp: i64) {
    let mut clock = svm.base().get_sysvar::<Clock>();
    clock.unix_timestamp = unix_timestamp;
    svm.set_sysvar(&clock);
}

fn permission_pda(account: &Address) -> Address {
    EphemeralPermission::find_pda(account).0
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

fn create_mint_ixs(svm: &MagicSVM, payer: Address, mint: Address) -> Vec<Instruction> {
    let mut initialize = vec![0u8, 0];
    initialize.extend_from_slice(payer.as_ref());
    initialize.push(0); // no freeze authority
    vec![
        system_instruction::create_account(
            &payer,
            &mint,
            svm.minimum_balance_for_rent_exemption(MINT_SIZE),
            MINT_SIZE as u64,
            &TOKEN_PROGRAM_ID,
        ),
        Instruction {
            program_id: TOKEN_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(mint, false),
                AccountMeta::new_readonly(solana_sdk_ids::sysvar::rent::id(), false),
            ],
            data: initialize,
        },
    ]
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

/// The SDK's `delegateSpl`: move `amount` into `owner`'s ephemeral ATA and delegate
/// it, setting up the mint's vault first if this is the mint's first deposit.
fn deposit_ixs(
    payer: Address,
    owner: Address,
    mint: Address,
    amount: u64,
    validator: Address,
    init_vault: bool,
) -> Vec<Instruction> {
    let mut ixs = vec![InitializeEphemeralAtaBuilder {
        payer,
        user: owner,
        mint,
    }
    .instruction()];
    if init_vault {
        let (vault, _) = GlobalVault::find_pda(&mint);
        ixs.push(InitializeGlobalVaultBuilder { payer, mint }.instruction());
        ixs.push(create_ata_ix(payer, vault, mint));
        ixs.push(
            DelegateEphemeralAtaBuilder {
                payer,
                user: vault,
                mint,
                validator: Some(validator),
            }
            .instruction(),
        );
    }
    ixs.push(
        DepositSplTokensBuilder {
            authority: owner,
            user: owner,
            mint,
            amount,
        }
        .instruction(),
    );
    ixs.push(
        DelegateEphemeralAtaBuilder {
            payer,
            user: owner,
            mint,
            validator: Some(validator),
        }
        .instruction(),
    );
    ixs
}

fn expect_custom_error(
    svm: &mut MagicSVM,
    target: TransactionTarget,
    signers: &[&Keypair],
    ixs: &[Instruction],
    code: u32,
) {
    let tx = sign_tx(signers, ixs, svm.latest_blockhash_for(target));
    let failed = svm
        .send_transaction_to(target, tx)
        .expect_err("transaction should fail");
    svm.expire_blockhash_for(target);
    assert_eq!(
        failed.err,
        TransactionError::InstructionError(0, InstructionError::Custom(code)),
        "logs: {:#?}",
        failed.meta.logs
    );
}

fn end_ix(
    auction_id: u64,
    auctioneer: &Address,
    auction: &Address,
    bids: &[Address],
) -> Instruction {
    let mut end = ix(
        instruction::EndAuction { auction_id },
        accounts::EndAuction {
            auctioneer: pk(auctioneer),
            auction: pk(auction),
        },
    );
    end.accounts
        .extend(bids.iter().map(|bid| AccountMeta::new(*bid, false)));
    end
}

/// An auctioneer with SOL, a Token A lot in their wallet, a Token B mint, and the
/// auction's addresses.
struct Fixture {
    svm: MagicSVM,
    auctioneer: Keypair,
    auction_id: u64,
    token_a: Address,
    token_b: Address,
    auction: Address,
    seller_token_a: Address,
}

impl Fixture {
    fn new(auction_id: u64) -> Self {
        let auctioneer = Keypair::new();
        let mut svm = boot(
            env!("CARGO_MANIFEST_DIR"),
            "sealed_auction",
            as_address(program_id()),
            &auctioneer,
            10 * LAMPORTS_PER_SOL,
        );
        set_time(&mut svm, START_TS);
        let token_a = Keypair::new();
        let token_b = Keypair::new();
        let seller_token_a = ata(&auctioneer.pubkey(), &token_a.pubkey());
        let mut ixs = create_mint_ixs(&svm, auctioneer.pubkey(), token_a.pubkey());
        ixs.extend(create_mint_ixs(&svm, auctioneer.pubkey(), token_b.pubkey()));
        ixs.push(create_ata_ix(
            auctioneer.pubkey(),
            auctioneer.pubkey(),
            token_a.pubkey(),
        ));
        ixs.push(mint_to_ix(
            token_a.pubkey(),
            seller_token_a,
            auctioneer.pubkey(),
            LOT_AMOUNT,
        ));
        send_ixs(
            &mut svm,
            TransactionTarget::Base,
            &[&auctioneer, &token_a, &token_b],
            ixs,
            "create mints",
        );
        let auction = as_address(
            Pubkey::find_program_address(
                &[
                    AUCTION_SEED,
                    auctioneer.pubkey().as_ref(),
                    &auction_id.to_le_bytes(),
                ],
                &program_id(),
            )
            .0,
        );
        Self {
            svm,
            auctioneer,
            auction_id,
            token_a: token_a.pubkey(),
            token_b: token_b.pubkey(),
            auction,
            seller_token_a,
        }
    }

    fn initialize_ix(&self, lot_amount: u64, deadline_ts: i64) -> Instruction {
        let eata = Address::find_program_address(
            &[self.auction.as_ref(), self.token_b.as_ref()],
            &ESPL_TOKEN_PROGRAM_ID,
        )
        .0;
        let eata_buffer =
            Address::find_program_address(&[b"buffer", eata.as_ref()], &ESPL_TOKEN_PROGRAM_ID).0;
        ix(
            instruction::InitializeAuction {
                auction_id: self.auction_id,
                lot_amount,
                deadline_ts,
                sponsor_lamports: SPONSOR_LAMPORTS,
            },
            accounts::InitializeAuction {
                auctioneer: pk(&self.auctioneer.pubkey()),
                token_a_mint: pk(&self.token_a),
                token_b_mint: pk(&self.token_b),
                auction: pk(&self.auction),
                auction_token_a_account: pk(&ata(&self.auction, &self.token_a)),
                auction_token_b_account: pk(&ata(&self.auction, &self.token_b)),
                auction_token_b_ephemeral_ata: pk(&eata),
                auction_token_b_eata_buffer: pk(&eata_buffer),
                auction_token_b_eata_record: delegation_record_pda_from_delegated_account(&pk(
                    &eata,
                )),
                auction_token_b_eata_metadata: delegation_metadata_pda_from_delegated_account(&pk(
                    &eata,
                )),
                seller_token_a_account: pk(&self.seller_token_a),
                ephemeral_token_program: pk(&ESPL_TOKEN_PROGRAM_ID),
                delegation_program: pk(&DELEGATION_PROGRAM_ID),
                validator: Some(pk(&self.svm.validator_identity())),
                token_program: pk(&TOKEN_PROGRAM_ID),
                associated_token_program: pk(&ASSOCIATED_TOKEN_PROGRAM_ID),
                system_program: system_program::ID,
            },
        )
    }

    fn initialize(&mut self, deadline_ts: i64) {
        let initialize = self.initialize_ix(LOT_AMOUNT, deadline_ts);
        send_ixs(
            &mut self.svm,
            TransactionTarget::Base,
            &[&self.auctioneer],
            [initialize],
            "initialize auction",
        );
    }

    fn end_ix(&self, bids: &[Address]) -> Instruction {
        end_ix(
            self.auction_id,
            &self.auctioneer.pubkey(),
            &self.auction,
            bids,
        )
    }

    fn reclaim_ix(&self) -> Instruction {
        ix(
            instruction::ReclaimUnsoldLot {
                auction_id: self.auction_id,
            },
            accounts::ReclaimUnsoldLot {
                auction: pk(&self.auction),
                token_a_mint: pk(&self.token_a),
                auction_token_a_account: pk(&ata(&self.auction, &self.token_a)),
                seller_token_a_account: pk(&self.seller_token_a),
                token_program: pk(&TOKEN_PROGRAM_ID),
            },
        )
    }
}

#[test]
fn initialize_moves_the_token_a_lot_into_l1_custody() {
    let mut fixture = Fixture::new(1);
    fixture.initialize(START_TS + 60);
    let svm = &fixture.svm;

    let auction = auction_state(svm, TransactionTarget::Base, &fixture.auction);
    assert_eq!(auction.auctioneer, pk(&fixture.auctioneer.pubkey()));
    assert_eq!(auction.auction_id, 1);
    assert_eq!(auction.token_a_mint, pk(&fixture.token_a));
    assert_eq!(auction.token_b_mint, pk(&fixture.token_b));
    assert_eq!(auction.lot_amount, LOT_AMOUNT);
    assert_eq!(auction.deadline_ts, START_TS + 60);
    assert_eq!(auction.bid_count, 0);
    assert_eq!(auction.closed_bid_count, 0);
    assert_eq!(auction.status, AuctionStatus::Open);
    assert!(!auction.lot_claimed);
    assert!(svm.get_balance(&fixture.auction).unwrap() >= SPONSOR_LAMPORTS);
    assert_eq!(
        token_balance(svm, TransactionTarget::Base, &fixture.seller_token_a),
        0
    );
    assert_eq!(
        token_balance(
            svm,
            TransactionTarget::Base,
            &ata(&fixture.auction, &fixture.token_a)
        ),
        LOT_AMOUNT
    );
    // The auction's Token B escrow is live on the ER, empty.
    assert_eq!(
        token_balance(
            svm,
            TransactionTarget::Ephemeral,
            &ata(&fixture.auction, &fixture.token_b)
        ),
        0
    );
}

#[test]
fn rejects_invalid_initialization_and_early_reclaim() {
    let mut fixture = Fixture::new(2);
    let auctioneer = fixture.auctioneer.insecure_clone();

    let zero_lot = fixture.initialize_ix(0, START_TS + 60);
    expect_custom_error(
        &mut fixture.svm,
        TransactionTarget::Base,
        &[&auctioneer],
        &[zero_lot],
        INVALID_AMOUNT,
    );
    let expired = fixture.initialize_ix(LOT_AMOUNT, START_TS - 1);
    expect_custom_error(
        &mut fixture.svm,
        TransactionTarget::Base,
        &[&auctioneer],
        &[expired],
        DEADLINE_IN_PAST,
    );

    fixture.initialize(START_TS + 60);
    let reclaim = fixture.reclaim_ix();
    expect_custom_error(
        &mut fixture.svm,
        TransactionTarget::Base,
        &[&auctioneer],
        &[reclaim],
        AUCTION_NOT_ENDED,
    );
    let end = fixture.end_ix(&[]);
    expect_custom_error(
        &mut fixture.svm,
        TransactionTarget::Base,
        &[&auctioneer],
        &[end],
        AUCTION_STILL_OPEN,
    );
}

#[test]
fn ends_an_empty_auction_and_lets_the_seller_reclaim_the_lot() {
    let mut fixture = Fixture::new(3);
    fixture.initialize(START_TS + 1);
    set_time(&mut fixture.svm, START_TS + 1);

    let auctioneer = fixture.auctioneer.insecure_clone();
    let end = fixture.end_ix(&[]);
    send_ixs(
        &mut fixture.svm,
        TransactionTarget::Base,
        &[&auctioneer],
        [end],
        "end auction",
    );
    let ended = auction_state(&fixture.svm, TransactionTarget::Base, &fixture.auction);
    assert_eq!(ended.status, AuctionStatus::Ended);
    assert_eq!(ended.highest_bid, 0);
    assert_eq!(ended.highest_bidder, Pubkey::default());

    let reclaim = fixture.reclaim_ix();
    send_ixs(
        &mut fixture.svm,
        TransactionTarget::Base,
        &[&auctioneer],
        [reclaim],
        "reclaim unsold lot",
    );
    let settled = auction_state(&fixture.svm, TransactionTarget::Base, &fixture.auction);
    assert_eq!(settled.status, AuctionStatus::Settled);
    assert!(settled.lot_claimed);
    assert_eq!(
        token_balance(
            &fixture.svm,
            TransactionTarget::Base,
            &fixture.seller_token_a
        ),
        LOT_AMOUNT
    );
    assert_eq!(
        token_balance(
            &fixture.svm,
            TransactionTarget::Base,
            &ata(&fixture.auction, &fixture.token_a)
        ),
        0
    );
}

#[test]
fn private_bids_settle_refund_undelegate_and_finalize() {
    let mut fixture = Fixture::new(4);
    fixture.initialize(START_TS + 90);
    let Fixture {
        svm,
        auctioneer,
        auction_id,
        token_a,
        token_b,
        auction,
        ..
    } = &mut fixture;
    let (auction_id, token_a, token_b, auction) = (*auction_id, *token_a, *token_b, *auction);
    let validator = svm.validator_identity();
    let bidder_one = Keypair::new();
    let bidder_two = Keypair::new();
    let auction_token_b = ata(&auction, &token_b);
    let bidder_one_token_b = ata(&bidder_one.pubkey(), &token_b);
    let bidder_two_token_b = ata(&bidder_two.pubkey(), &token_b);
    let seller_token_b = ata(&auctioneer.pubkey(), &token_b);
    let winner_token_a = ata(&bidder_two.pubkey(), &token_a);

    // Fund the bidders and the ESPL rent PDA; give each bidder their Token B and the
    // seller 1 Token B so its ER token account exists; open the winner's Token A account.
    send_ixs(
        svm,
        TransactionTarget::Base,
        &[auctioneer],
        [
            system_instruction::transfer(
                &auctioneer.pubkey(),
                &bidder_one.pubkey(),
                LAMPORTS_PER_SOL,
            ),
            system_instruction::transfer(
                &auctioneer.pubkey(),
                &bidder_two.pubkey(),
                LAMPORTS_PER_SOL,
            ),
            system_instruction::transfer(
                &auctioneer.pubkey(),
                &find_rent_pda().0,
                LAMPORTS_PER_SOL / 5,
            ),
            create_ata_ix(auctioneer.pubkey(), bidder_one.pubkey(), token_b),
            create_ata_ix(auctioneer.pubkey(), bidder_two.pubkey(), token_b),
            create_ata_ix(auctioneer.pubkey(), auctioneer.pubkey(), token_b),
            create_ata_ix(auctioneer.pubkey(), bidder_two.pubkey(), token_a),
            mint_to_ix(token_b, bidder_one_token_b, auctioneer.pubkey(), BID_ONE),
            mint_to_ix(token_b, bidder_two_token_b, auctioneer.pubkey(), BID_TWO),
            mint_to_ix(token_b, seller_token_b, auctioneer.pubkey(), 1),
        ],
        "fund bidders",
    );

    let auction_pk = pk(&auction);
    send_ixs(
        svm,
        TransactionTarget::Base,
        &[auctioneer],
        [ix(
            instruction::DelegateAuction { auction_id },
            accounts::DelegateAuction {
                auctioneer: pk(&auctioneer.pubkey()),
                buffer_auction: delegate_buffer_pda_from_delegated_account_and_owner_program(
                    &auction_pk,
                    &program_id(),
                ),
                delegation_record_auction: delegation_record_pda_from_delegated_account(
                    &auction_pk,
                ),
                delegation_metadata_auction: delegation_metadata_pda_from_delegated_account(
                    &auction_pk,
                ),
                auction: auction_pk,
                validator: Some(pk(&validator)),
                owner_program: program_id(),
                delegation_program: DelegationProgram::id(),
                system_program: system_program::ID,
            },
        )],
        "delegate auction",
    );

    for (owner, amount, init_vault) in [
        (&bidder_one, BID_ONE, true),
        (&bidder_two, BID_TWO, false),
        (&*auctioneer, 1, false),
    ] {
        send_ixs(
            svm,
            TransactionTarget::Base,
            &[auctioneer, owner],
            deposit_ixs(
                auctioneer.pubkey(),
                owner.pubkey(),
                token_b,
                amount,
                validator,
                init_vault,
            ),
            "deposit Token B",
        );
    }

    // Each bidder places a bid on the ER (the auction sponsors the ephemeral bid
    // account), then the auction makes it private to the auctioneer and the bidder.
    let bid_of = |bidder: &Keypair| {
        as_address(
            Pubkey::find_program_address(
                &[BID_SEED, auction.as_ref(), bidder.pubkey().as_ref()],
                &program_id(),
            )
            .0,
        )
    };
    let bid_one = bid_of(&bidder_one);
    let bid_two = bid_of(&bidder_two);
    for (bidder, amount, bid, bidder_token_b) in [
        (&bidder_one, BID_ONE, bid_one, bidder_one_token_b),
        (&bidder_two, BID_TWO, bid_two, bidder_two_token_b),
    ] {
        send_ixs(
            svm,
            TransactionTarget::Ephemeral,
            &[auctioneer, bidder],
            [ix(
                instruction::PlaceBid { auction_id, amount },
                accounts::PlaceBid {
                    payer: pk(&auctioneer.pubkey()),
                    bidder: pk(&bidder.pubkey()),
                    token_b_mint: pk(&token_b),
                    auction: auction_pk,
                    bid: pk(&bid),
                    bidder_token_b_account: pk(&bidder_token_b),
                    auction_token_b_account: pk(&auction_token_b),
                    token_program: pk(&TOKEN_PROGRAM_ID),
                    vault: pk(&EPHEMERAL_VAULT_ID),
                    magic_program: pk(&MAGIC_PROGRAM_ID),
                },
            )],
            "place bid",
        );
        send_ixs(
            svm,
            TransactionTarget::Ephemeral,
            &[auctioneer],
            [ix(
                instruction::InitBidPermission { auction_id },
                accounts::InitBidPermission {
                    auction: auction_pk,
                    bid: pk(&bid),
                    bid_permission: pk(&permission_pda(&bid)),
                    permission_program: pk(&PERMISSION_PROGRAM_ID),
                    ephemeral_vault: pk(&EPHEMERAL_VAULT_ID),
                    magic_program: pk(&MAGIC_PROGRAM_ID),
                },
            )],
            "init bid permission",
        );
    }

    // The permission is private, with the auctioneer and the bidder as members.
    for (bid, bidder) in [(bid_one, &bidder_one), (bid_two, &bidder_two)] {
        let permission = svm
            .ephemeral()
            .get_account(&permission_pda(&bid))
            .expect("bid permission on the ER");
        assert_eq!(permission.owner, PERMISSION_PROGRAM_ID);
        let permission = EphemeralPermission::from_bytes(&permission.data).unwrap();
        assert_eq!(permission.permissioned_account, bid);
        assert!(permission.private);
        let members: Vec<Address> = permission.members.iter().map(|m| m.pubkey).collect();
        assert!(members.contains(&auctioneer.pubkey()));
        assert!(members.contains(&bidder.pubkey()));
    }

    // Only members can read a private bid on the ER.
    let can_read = |svm: &mut MagicSVM, viewer: Option<Address>, bid: &Address| {
        svm.set_authorized_user(viewer);
        svm.get_account_for(TransactionTarget::Ephemeral, bid)
            .is_some()
    };
    assert!(!can_read(svm, None, &bid_one));
    assert!(!can_read(svm, Some(bidder_two.pubkey()), &bid_one));
    assert!(can_read(svm, Some(bidder_two.pubkey()), &bid_two));
    assert!(can_read(svm, Some(bidder_one.pubkey()), &bid_one));
    assert!(can_read(svm, Some(auctioneer.pubkey()), &bid_one));
    assert!(can_read(svm, Some(auctioneer.pubkey()), &bid_two));
    // The auction itself has no permission and stays public.
    svm.set_authorized_user(None);
    let state = auction_state(svm, TransactionTarget::Ephemeral, &auction);
    assert_eq!(state.bid_count, 2);
    assert_eq!(state.closed_bid_count, 0);
    assert_eq!(
        token_balance(svm, TransactionTarget::Ephemeral, &auction_token_b),
        BID_ONE + BID_TWO
    );
    svm.set_authorized_user(Some(auctioneer.pubkey()));

    // After the deadline, the auctioneer ends the auction over every bid.
    set_time(svm, START_TS + 90);
    let end = end_ix(
        auction_id,
        &auctioneer.pubkey(),
        &auction,
        &[bid_one, bid_two],
    );
    send_ixs(
        svm,
        TransactionTarget::Ephemeral,
        &[auctioneer],
        [end],
        "end auction",
    );
    let ended = auction_state(svm, TransactionTarget::Ephemeral, &auction);
    assert_eq!(ended.status, AuctionStatus::Ended);
    assert_eq!(ended.highest_bid, BID_TWO);
    assert_eq!(ended.highest_bidder, pk(&bidder_two.pubkey()));

    // The winner can't claim a refund.
    let refund = |bidder: &Keypair, bid: Address, bidder_token_b: Address| {
        ix(
            instruction::ClaimRefund {},
            accounts::ClaimRefund {
                bidder: pk(&bidder.pubkey()),
                auction: auction_pk,
                bid: pk(&bid),
                token_b_mint: pk(&token_b),
                auction_token_b_account: pk(&auction_token_b),
                bidder_token_b_account: pk(&bidder_token_b),
                bid_permission: pk(&permission_pda(&bid)),
                permission_program: pk(&PERMISSION_PROGRAM_ID),
                magic_program: pk(&MAGIC_PROGRAM_ID),
                token_program: pk(&TOKEN_PROGRAM_ID),
                vault: pk(&EPHEMERAL_VAULT_ID),
            },
        )
    };
    let winner_refund = refund(&bidder_two, bid_two, bidder_two_token_b);
    expect_custom_error(
        svm,
        TransactionTarget::Ephemeral,
        &[auctioneer],
        &[winner_refund],
        WINNER_CANNOT_REFUND,
    );

    // Settle the winning bid: the seller receives it, the bid and its permission close.
    send_ixs(
        svm,
        TransactionTarget::Ephemeral,
        &[auctioneer],
        [ix(
            instruction::SettleWinningBid {},
            accounts::SettleWinningBid {
                crank: pk(&auctioneer.pubkey()),
                auction: auction_pk,
                winning_bid: pk(&bid_two),
                token_b_mint: pk(&token_b),
                auction_token_b_account: pk(&auction_token_b),
                seller_token_b_account: pk(&seller_token_b),
                bid_permission: pk(&permission_pda(&bid_two)),
                permission_program: pk(&PERMISSION_PROGRAM_ID),
                magic_program: pk(&MAGIC_PROGRAM_ID),
                token_program: pk(&TOKEN_PROGRAM_ID),
                vault: pk(&EPHEMERAL_VAULT_ID),
            },
        )],
        "settle winning bid",
    );
    assert_eq!(
        token_balance(svm, TransactionTarget::Ephemeral, &seller_token_b),
        BID_TWO + 1
    );
    assert!(svm
        .get_account_for(TransactionTarget::Ephemeral, &bid_two)
        .is_none());
    assert!(svm
        .ephemeral()
        .get_account(&permission_pda(&bid_two))
        .is_none());

    // The loser gets their bid back.
    send_ixs(
        svm,
        TransactionTarget::Ephemeral,
        &[auctioneer],
        [refund(&bidder_one, bid_one, bidder_one_token_b)],
        "claim refund",
    );
    let refunded = auction_state(svm, TransactionTarget::Ephemeral, &auction);
    assert_eq!(refunded.closed_bid_count, 2);
    assert_eq!(
        token_balance(svm, TransactionTarget::Ephemeral, &bidder_one_token_b),
        BID_ONE
    );
    assert_eq!(
        token_balance(svm, TransactionTarget::Ephemeral, &auction_token_b),
        0
    );
    assert!(svm
        .get_account_for(TransactionTarget::Ephemeral, &bid_one)
        .is_none());
    assert!(svm
        .ephemeral()
        .get_account(&permission_pda(&bid_one))
        .is_none());

    // Commit and undelegate the auction, then the winner's lot is released on L1.
    send_ixs(
        svm,
        TransactionTarget::Ephemeral,
        &[auctioneer],
        [ix(
            instruction::UndelegateAuction { auction_id },
            accounts::UndelegateAuction {
                payer: pk(&auctioneer.pubkey()),
                auction: auction_pk,
                magic_program: MagicProgram::id(),
                magic_context: MAGIC_CONTEXT_ID,
            },
        )],
        "undelegate auction",
    );
    assert_eq!(
        svm.get_account_for(TransactionTarget::Base, &auction)
            .unwrap()
            .owner,
        as_address(program_id())
    );
    let committed = auction_state(svm, TransactionTarget::Base, &auction);
    assert_eq!(committed.status, AuctionStatus::Ended);
    assert_eq!(committed.highest_bidder, pk(&bidder_two.pubkey()));

    send_ixs(
        svm,
        TransactionTarget::Base,
        &[auctioneer],
        [ix(
            instruction::Finalize { auction_id },
            accounts::Finalize {
                auction: auction_pk,
                token_a_mint: pk(&token_a),
                auction_token_a_account: pk(&ata(&auction, &token_a)),
                winner_token_a_account: pk(&winner_token_a),
                token_program: pk(&TOKEN_PROGRAM_ID),
            },
        )],
        "finalize",
    );
    let finalized = auction_state(svm, TransactionTarget::Base, &auction);
    assert_eq!(finalized.status, AuctionStatus::Settled);
    assert!(finalized.lot_claimed);
    assert_eq!(
        token_balance(svm, TransactionTarget::Base, &winner_token_a),
        LOT_AMOUNT
    );
}

#[test]
fn matches_the_fixed_instruction_surface() {
    let path =
        test_utils::example_dir(env!("CARGO_MANIFEST_DIR")).join("target/idl/sealed_auction.json");
    let idl: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut names: Vec<&str> = idl["instructions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ix| ix["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "claim_refund",
            "delegate_auction",
            "end_auction",
            "finalize",
            "init_auction_permission",
            "init_bid_permission",
            "initialize_auction",
            "place_bid",
            "process_undelegation",
            "reclaim_unsold_lot",
            "settle_winning_bid",
            "undelegate_auction",
        ]
    );
}
