//! SpotTokenBalance Integration Tests (Dynamic Token Balance Architecture)
//!
//! Tests the new per-token PDA system that replaces SpotUserAccount.
//! Covers: RelayerSpotDeposit, RelayerSpotWithdraw, RelayerSpotSettleTrade,
//!         auto-init, self-trade, insufficient balance, fee boundaries,
//!         and conservation invariant.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_instruction,
    system_program,
};
use solana_program_test::*;
use solana_sdk::{
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use vault_program::{
    instruction::VaultInstruction,
    state::*,
};

fn derive_balance_pda(program_id: &Pubkey, wallet: &Pubkey, token_index: u16) -> (Pubkey, u8) {
    derive_balance_pda_with_index(program_id, wallet, 0, token_index)
}

fn derive_balance_pda_with_index(program_id: &Pubkey, wallet: &Pubkey, account_index: u8, token_index: u16) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"spot_balance", wallet.as_ref(), &[account_index], &token_index.to_le_bytes()],
        program_id,
    )
}

fn derive_vault_config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault_config"], program_id)
}

async fn read_spot_balance(
    banks_client: &mut BanksClient,
    pda: &Pubkey,
) -> Option<SpotTokenBalance> {
    banks_client
        .get_account(*pda)
        .await
        .unwrap()
        .map(|acc| SpotTokenBalance::try_from_slice(&acc.data).unwrap())
}

async fn setup_vault(program_id: &Pubkey) -> ProgramTest {
    let mut program_test = ProgramTest::new(
        "vault_program",
        *program_id,
        processor!(vault_program::processor::process_instruction),
    );
    program_test
}

async fn initialize_vault_config(
    banks_client: &mut BanksClient,
    payer: &Keypair,
    program_id: &Pubkey,
) {
    let (vault_config_pda, _) = derive_vault_config_pda(program_id);

    let ledger = Pubkey::new_unique();
    let delegation = Pubkey::new_unique();
    let fund = Pubkey::new_unique();
    let usdc_mint = Pubkey::new_unique();
    let vault_token_account = Pubkey::new_unique();

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(vault_config_pda, false),
            AccountMeta::new_readonly(usdc_mint, false),
            AccountMeta::new_readonly(vault_token_account, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: VaultInstruction::Initialize {
            ledger_program: ledger,
            delegation_program: delegation,
            fund_program: fund,
        }
        .try_to_vec()
        .unwrap(),
    };

    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer],
        recent_blockhash,
    );
    banks_client.process_transaction(tx).await.unwrap();
}

fn build_relayer_spot_deposit_ix(
    program_id: &Pubkey,
    admin: &Pubkey,
    user_wallet: &Pubkey,
    token_index: u16,
    amount: u64,
) -> Instruction {
    let (balance_pda, _) = derive_balance_pda(program_id, user_wallet, token_index);
    let (vault_config_pda, _) = derive_vault_config_pda(program_id);

    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*admin, true),
            AccountMeta::new(balance_pda, false),
            AccountMeta::new_readonly(vault_config_pda, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: VaultInstruction::RelayerSpotDeposit {
            user_wallet: *user_wallet,
            token_index,
            amount,
            account_index: 0,
        }
        .try_to_vec()
        .unwrap(),
    }
}

fn build_relayer_spot_withdraw_ix(
    program_id: &Pubkey,
    admin: &Pubkey,
    user_wallet: &Pubkey,
    token_index: u16,
    amount: u64,
) -> Instruction {
    let (balance_pda, _) = derive_balance_pda(program_id, user_wallet, token_index);
    let (vault_config_pda, _) = derive_vault_config_pda(program_id);

    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*admin, true),
            AccountMeta::new(balance_pda, false),
            AccountMeta::new_readonly(vault_config_pda, false),
        ],
        data: VaultInstruction::RelayerSpotWithdraw {
            user_wallet: *user_wallet,
            token_index,
            amount,
            account_index: 0,
        }
        .try_to_vec()
        .unwrap(),
    }
}

fn build_relayer_settle_ix(
    program_id: &Pubkey,
    admin: &Pubkey,
    maker_wallet: &Pubkey,
    taker_wallet: &Pubkey,
    base_token_index: u16,
    quote_token_index: u16,
    base_amount_e6: i64,
    quote_amount_e6: i64,
    maker_fee_e6: i64,
    taker_fee_e6: i64,
    taker_is_buy: bool,
) -> Instruction {
    let (maker_base_pda, _) = derive_balance_pda(program_id, maker_wallet, base_token_index);
    let (maker_quote_pda, _) = derive_balance_pda(program_id, maker_wallet, quote_token_index);
    let (taker_base_pda, _) = derive_balance_pda(program_id, taker_wallet, base_token_index);
    let (taker_quote_pda, _) = derive_balance_pda(program_id, taker_wallet, quote_token_index);
    let (vault_config_pda, _) = derive_vault_config_pda(program_id);

    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*admin, true),
            AccountMeta::new(maker_base_pda, false),
            AccountMeta::new(maker_quote_pda, false),
            AccountMeta::new(taker_base_pda, false),
            AccountMeta::new(taker_quote_pda, false),
            AccountMeta::new_readonly(vault_config_pda, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: VaultInstruction::RelayerSpotSettleTrade {
            maker_wallet: *maker_wallet,
            taker_wallet: *taker_wallet,
            base_token_index,
            quote_token_index,
            base_amount_e6,
            quote_amount_e6,
            maker_fee_e6,
            taker_fee_e6,
            taker_is_buy,
            sequence: 1,
            maker_account_index: 0,
            taker_account_index: 0,
        }
        .try_to_vec()
        .unwrap(),
    }
}

// ============================================================
// Test 5.3: Deposit + Auto-Init
// ============================================================
#[tokio::test]
async fn test_relayer_spot_deposit_auto_init() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let user = Pubkey::new_unique();
    let token_index: u16 = 1; // wBTC
    let (balance_pda, _) = derive_balance_pda(&program_id, &user, token_index);

    // PDA should not exist yet
    assert!(read_spot_balance(&mut banks_client, &balance_pda).await.is_none());

    // First deposit: auto-init + 1000 units
    let ix = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &user, token_index, 1000_000_000);
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], recent_blockhash);
    banks_client.process_transaction(tx).await.unwrap();

    // Verify PDA created with correct balance
    let balance = read_spot_balance(&mut banks_client, &balance_pda).await.unwrap();
    assert_eq!(balance.discriminator, SPOT_TOKEN_BALANCE_DISCRIMINATOR);
    assert_eq!(balance.wallet, user);
    assert_eq!(balance.token_index, token_index);
    assert_eq!(balance.available_e6, 1000_000_000);
    assert_eq!(balance.locked_e6, 0);

    // Second deposit: should NOT re-create, just add
    let ix2 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &user, token_index, 500_000_000);
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let tx2 = Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], recent_blockhash);
    banks_client.process_transaction(tx2).await.unwrap();

    let balance2 = read_spot_balance(&mut banks_client, &balance_pda).await.unwrap();
    assert_eq!(balance2.available_e6, 1500_000_000); // 1000 + 500
}

// ============================================================
// Test 5.4: Withdraw success + insufficient
// ============================================================
#[tokio::test]
async fn test_relayer_spot_withdraw() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let user = Pubkey::new_unique();
    let token_index: u16 = 1; // non-USDC (token_index=0 must use Vault path)

    // Deposit 1000
    let ix = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &user, token_index, 1000_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Withdraw 400 (should succeed)
    let ix2 = build_relayer_spot_withdraw_ix(&program_id, &payer.pubkey(), &user, token_index, 400_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let (balance_pda, _) = derive_balance_pda(&program_id, &user, token_index);
    let balance = read_spot_balance(&mut banks_client, &balance_pda).await.unwrap();
    assert_eq!(balance.available_e6, 600_000_000); // 1000 - 400

    // Withdraw 700 (should fail — only 600 available)
    let ix3 = build_relayer_spot_withdraw_ix(&program_id, &payer.pubkey(), &user, token_index, 700_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    let result = banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix3], Some(&payer.pubkey()), &[&payer], bh)).await;
    assert!(result.is_err()); // InsufficientBalance
}

// ============================================================
// Test 5.7: Settle normal path (taker buy)
// Pre-create ALL 4 PDAs to avoid auto-init complexity
// ============================================================
#[tokio::test]
async fn test_settle_taker_buy() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let maker = Pubkey::new_unique();
    let taker = Pubkey::new_unique();
    let base_token: u16 = 1; // wBTC
    let quote_token: u16 = 0; // USDC

    // Pre-create ALL 4 PDAs via deposit
    let ix1 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &maker, base_token, 10_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix1], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix2 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &maker, quote_token, 0);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix3 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &taker, base_token, 0);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix3], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix4 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &taker, quote_token, 50000_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix4], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Settle: taker buys 1 wBTC for 30000 USDC, fees: maker=60, taker=150
    let settle_ix = build_relayer_settle_ix(
        &program_id, &payer.pubkey(), &maker, &taker,
        base_token, quote_token,
        1_000_000, 30000_000_000, 60_000_000, 150_000_000, true,
    );
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[settle_ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Verify results
    let (maker_base_pda, _) = derive_balance_pda(&program_id, &maker, base_token);
    let maker_base = read_spot_balance(&mut banks_client, &maker_base_pda).await.unwrap();
    assert_eq!(maker_base.available_e6, 9_000_000); // 10 - 1

    let (maker_quote_pda, _) = derive_balance_pda(&program_id, &maker, quote_token);
    let maker_quote = read_spot_balance(&mut banks_client, &maker_quote_pda).await.unwrap();
    assert_eq!(maker_quote.available_e6, 29940_000_000); // 0 + (30000 - 60)

    let (taker_base_pda, _) = derive_balance_pda(&program_id, &taker, base_token);
    let taker_base = read_spot_balance(&mut banks_client, &taker_base_pda).await.unwrap();
    assert_eq!(taker_base.available_e6, 1_000_000); // 0 + 1

    let (taker_quote_pda, _) = derive_balance_pda(&program_id, &taker, quote_token);
    let taker_quote = read_spot_balance(&mut banks_client, &taker_quote_pda).await.unwrap();
    assert_eq!(taker_quote.available_e6, 19850_000_000); // 50000 - 30000 - 150
}

// ============================================================
// Test 5.8: Settle normal path (taker sell)
// ============================================================
#[tokio::test]
async fn test_settle_taker_sell() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let maker = Pubkey::new_unique();
    let taker = Pubkey::new_unique();
    let base_token: u16 = 1;
    let quote_token: u16 = 0;

    // Pre-create ALL 4 PDAs — maker buys, so maker needs enough USDC
    let ix1 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &maker, quote_token, 100000_000_000); // 100000 USDC
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix1], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix2 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &maker, base_token, 0);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix3 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &taker, base_token, 5_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix3], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix4 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &taker, quote_token, 0);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix4], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Settle: taker sells 2 wBTC for 60000 USDC. Maker buys, pays 60000+120(fee)=60120 USDC
    let settle_ix = build_relayer_settle_ix(
        &program_id, &payer.pubkey(), &maker, &taker,
        base_token, quote_token,
        2_000_000, 60000_000_000, 120_000_000, 300_000_000, false,
    );
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[settle_ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Maker: bought 2 wBTC (auto-init), paid 60000+120=60120 USDC
    let (maker_base_pda, _) = derive_balance_pda(&program_id, &maker, base_token);
    let maker_base = read_spot_balance(&mut banks_client, &maker_base_pda).await.unwrap();
    assert_eq!(maker_base.available_e6, 2_000_000);
    let (maker_quote_pda, _) = derive_balance_pda(&program_id, &maker, quote_token);
    let maker_quote = read_spot_balance(&mut banks_client, &maker_quote_pda).await.unwrap();
    assert_eq!(maker_quote.available_e6, 100000_000_000 - 60000_000_000 - 120_000_000); // 100000 - 60000 - 120 = 39880

    // Taker: sold 2 wBTC, received 60000-300=59700 USDC (auto-init)
    let (taker_base_pda, _) = derive_balance_pda(&program_id, &taker, base_token);
    let taker_base = read_spot_balance(&mut banks_client, &taker_base_pda).await.unwrap();
    assert_eq!(taker_base.available_e6, 3_000_000); // 5 - 2
    let (taker_quote_pda, _) = derive_balance_pda(&program_id, &taker, quote_token);
    let taker_quote = read_spot_balance(&mut banks_client, &taker_quote_pda).await.unwrap();
    assert_eq!(taker_quote.available_e6, 59700_000_000);
}

// ============================================================
// Test 5.9: Settle self-trade
// ============================================================
#[tokio::test]
async fn test_settle_self_trade() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let user = Pubkey::new_unique();
    let base_token: u16 = 1;
    let quote_token: u16 = 0;

    // Setup: user has 10 wBTC
    let ix1 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &user, base_token, 10_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix1], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Setup: user has 50000 USDC
    let ix2 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &user, quote_token, 50000_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Self-trade: maker == taker, fees = 50+100=150 USDC total
    let settle_ix = build_relayer_settle_ix(
        &program_id, &payer.pubkey(), &user, &user,
        base_token, quote_token,
        1_000_000, 30000_000_000, 50_000_000, 100_000_000, true,
    );
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[settle_ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Base: unchanged (buy+sell cancel out)
    let (base_pda, _) = derive_balance_pda(&program_id, &user, base_token);
    let base_bal = read_spot_balance(&mut banks_client, &base_pda).await.unwrap();
    assert_eq!(base_bal.available_e6, 10_000_000);

    // Quote: deducted total fees (50+100=150)
    let (quote_pda, _) = derive_balance_pda(&program_id, &user, quote_token);
    let quote_bal = read_spot_balance(&mut banks_client, &quote_pda).await.unwrap();
    assert_eq!(quote_bal.available_e6, 50000_000_000 - 150_000_000);
}

// ============================================================
// Test 5.10: Settle insufficient balance
// ============================================================
#[tokio::test]
async fn test_settle_insufficient_balance() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let maker = Pubkey::new_unique();
    let taker = Pubkey::new_unique();

    // Maker has 10 wBTC
    let ix1 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &maker, 1, 10_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix1], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Taker only has 100 USDC but needs 30000+150
    let ix2 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &taker, 0, 100_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let settle_ix = build_relayer_settle_ix(
        &program_id, &payer.pubkey(), &maker, &taker,
        1, 0, 1_000_000, 30000_000_000, 60_000_000, 150_000_000, true,
    );
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    let result = banks_client.process_transaction(Transaction::new_signed_with_payer(&[settle_ix], Some(&payer.pubkey()), &[&payer], bh)).await;
    assert!(result.is_err()); // SettlementFailed — insufficient quote balance
}

// ============================================================
// Test 5.13: Conservation invariant
// ============================================================
#[tokio::test]
async fn test_settle_conservation() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let maker = Pubkey::new_unique();
    let taker = Pubkey::new_unique();
    let base_token: u16 = 2; // wETH
    let quote_token: u16 = 0; // USDC

    // Pre-create ALL 4 PDAs
    let ix1 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &maker, base_token, 100_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix1], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix2 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &maker, quote_token, 0);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix3 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &taker, base_token, 0);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix3], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix4 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &taker, quote_token, 200000_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix4], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Pre-settle totals: base = 100M (maker), quote = 200B (taker)
    let total_base_before: i64 = 100_000_000;
    let total_quote_before: i64 = 200000_000_000;

    // Settle: taker buys 10 wETH for 20000 USDC, fees: maker=40, taker=100
    let settle_ix = build_relayer_settle_ix(
        &program_id, &payer.pubkey(), &maker, &taker,
        base_token, quote_token,
        10_000_000, 20000_000_000, 40_000_000, 100_000_000, true,
    );
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[settle_ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Post-settle: read all 4 PDAs
    let (mb, _) = derive_balance_pda(&program_id, &maker, base_token);
    let (mq, _) = derive_balance_pda(&program_id, &maker, quote_token);
    let (tb, _) = derive_balance_pda(&program_id, &taker, base_token);
    let (tq, _) = derive_balance_pda(&program_id, &taker, quote_token);

    let mb_bal = read_spot_balance(&mut banks_client, &mb).await.unwrap();
    let mq_bal = read_spot_balance(&mut banks_client, &mq).await.unwrap();
    let tb_bal = read_spot_balance(&mut banks_client, &tb).await.unwrap();
    let tq_bal = read_spot_balance(&mut banks_client, &tq).await.unwrap();

    let total_base_after = mb_bal.total().unwrap() + tb_bal.total().unwrap();
    let total_quote_after = mq_bal.total().unwrap() + tq_bal.total().unwrap();

    // Base conservation: 100M = 90M (maker) + 10M (taker)
    assert_eq!(total_base_after, total_base_before);

    // Quote: NOT conserved because fees are extracted
    // total_quote_after = (20000-40)M + (200000-20000-100)M = 19960M + 179900M = 199860M
    // total_quote_before = 200000M
    // difference = fees = 40+100 = 140M
    let fee_total = 40_000_000i64 + 100_000_000i64;
    assert_eq!(total_quote_before - total_quote_after, fee_total);
}

// ============================================================
// Test 5.11: Fee > quote_amount (taker sell path)
// ============================================================
#[tokio::test]
async fn test_settle_fee_exceeds_quote() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let maker = Pubkey::new_unique();
    let taker = Pubkey::new_unique();

    // Pre-create all 4 PDAs with enough balance
    for (w, t, a) in [
        (&maker, 0u16, 100000_000_000u64),
        (&maker, 1u16, 0u64),
        (&taker, 1u16, 10_000_000u64),
        (&taker, 0u16, 0u64),
    ] {
        let ix = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), w, t, a);
        let bh = banks_client.get_latest_blockhash().await.unwrap();
        banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();
    }

    // Settle: taker sells 1 wBTC for 100 USDC, but taker_fee=200 USDC (fee > quote)
    let settle_ix = build_relayer_settle_ix(
        &program_id, &payer.pubkey(), &maker, &taker,
        1, 0, 1_000_000, 100_000_000, 10_000_000, 200_000_000, false,
    );
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    let result = banks_client.process_transaction(
        Transaction::new_signed_with_payer(&[settle_ix], Some(&payer.pubkey()), &[&payer], bh)
    ).await;
    assert!(result.is_err()); // SettlementFailed — fee exceeds quote amount
}

// ============================================================
// Test 5.6: Allocate from Vault + Release to Vault
// ============================================================
#[tokio::test]
async fn test_allocate_and_release() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let user = Keypair::new();

    // Fund user with SOL
    let airdrop_ix = system_instruction::transfer(&payer.pubkey(), &user.pubkey(), 1_000_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[airdrop_ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Initialize UserAccount PDA (seeds: ["user", wallet, &[account_index]])
    let (user_account_pda, _) = UserAccount::derive_pda(&program_id, &user.pubkey(), 0);
    let init_user_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user.pubkey(), true),
            AccountMeta::new(user_account_pda, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: VaultInstruction::InitializeUser { account_index: 0 }.try_to_vec().unwrap(),
    };
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[init_user_ix], Some(&user.pubkey()), &[&user], bh)).await.unwrap();

    // Deposit USDC into UserAccount (Perp account) via RelayerDeposit
    let deposit_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(user_account_pda, false),
            AccountMeta::new_readonly(derive_vault_config_pda(&program_id).0, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: VaultInstruction::RelayerDeposit { user_wallet: user.pubkey(), amount: 5000_000_000, account_index: 0 }.try_to_vec().unwrap(),
    };
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[deposit_ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Verify UserAccount has 5000 USDC
    let ua_data = banks_client.get_account(user_account_pda).await.unwrap().unwrap();
    let ua = UserAccount::try_from_slice(&ua_data.data).unwrap();
    assert_eq!(ua.available_balance_e6, 5000_000_000);

    // Allocate 2000 USDC from UserAccount to SpotTokenBalance(USDC=0)
    let (usdc_balance_pda, _) = derive_balance_pda(&program_id, &user.pubkey(), 0);
    let allocate_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(user_account_pda, false),
            AccountMeta::new(usdc_balance_pda, false),
            AccountMeta::new_readonly(derive_vault_config_pda(&program_id).0, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: VaultInstruction::SpotAllocateFromVault { user_wallet: user.pubkey(), amount: 2000_000_000, account_index: 0 }.try_to_vec().unwrap(),
    };
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[allocate_ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Verify: UserAccount = 3000, SpotTokenBalance(USDC) = 2000
    let ua_data = banks_client.get_account(user_account_pda).await.unwrap().unwrap();
    let ua = UserAccount::try_from_slice(&ua_data.data).unwrap();
    assert_eq!(ua.available_balance_e6, 3000_000_000);

    let spot_usdc = read_spot_balance(&mut banks_client, &usdc_balance_pda).await.unwrap();
    assert_eq!(spot_usdc.available_e6, 2000_000_000);

    // Release 800 USDC from SpotTokenBalance back to UserAccount
    let release_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(usdc_balance_pda, false),
            AccountMeta::new(user_account_pda, false),
            AccountMeta::new_readonly(derive_vault_config_pda(&program_id).0, false),
        ],
        data: VaultInstruction::SpotReleaseToVault { user_wallet: user.pubkey(), amount: 800_000_000, account_index: 0 }.try_to_vec().unwrap(),
    };
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[release_ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    // Verify: UserAccount = 3800, SpotTokenBalance(USDC) = 1200
    let ua_data = banks_client.get_account(user_account_pda).await.unwrap().unwrap();
    let ua = UserAccount::try_from_slice(&ua_data.data).unwrap();
    assert_eq!(ua.available_balance_e6, 3800_000_000);

    let spot_usdc = read_spot_balance(&mut banks_client, &usdc_balance_pda).await.unwrap();
    assert_eq!(spot_usdc.available_e6, 1200_000_000);

    // Conservation: 3800 + 1200 = 5000 (original)
    assert_eq!(ua.available_balance_e6 + spot_usdc.available_e6, 5000_000_000);
}

// ============================================================
// Test: Deprecated InitializeSpotUser returns error
// ============================================================
#[tokio::test]
async fn test_deprecated_initialize_spot_user() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: VaultInstruction::Deprecated_InitializeSpotUser.try_to_vec().unwrap(),
    };

    let bh = banks_client.get_latest_blockhash().await.unwrap();
    let result = banks_client.process_transaction(
        Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], bh)
    ).await;
    assert!(result.is_err()); // DeprecatedInstruction
}
