//! SpotTokenBalance Integration Tests (Dynamic Token Balance Architecture)
//!
//! Tests the per-token PDA system.
//! Covers: RelayerSpotDeposit, RelayerSpotWithdraw,
//!         auto-init, and insufficient balance.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
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

    let delegation = Pubkey::new_unique();
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
            delegation_program: delegation,
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
    governance_authority: &Pubkey,
    user_wallet: &Pubkey,
    token_index: u16,
    amount: u64,
) -> Instruction {
    let (balance_pda, _) = derive_balance_pda(program_id, user_wallet, token_index);
    let (vault_config_pda, _) = derive_vault_config_pda(program_id);

    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*governance_authority, true),
            AccountMeta::new(balance_pda, false),
            AccountMeta::new_readonly(vault_config_pda, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: VaultInstruction::RelayerSpotDeposit {
            user_wallet: *user_wallet,
            token_index,
            amount,
            account_index: 0,
            amount_e6: amount as i64,
        }
        .try_to_vec()
        .unwrap(),
    }
}

fn build_relayer_spot_withdraw_ix(
    program_id: &Pubkey,
    governance_authority: &Pubkey,
    user_wallet: &Pubkey,
    token_index: u16,
    amount: u64,
) -> Instruction {
    let (balance_pda, _) = derive_balance_pda(program_id, user_wallet, token_index);
    let (vault_config_pda, _) = derive_vault_config_pda(program_id);

    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*governance_authority, true),
            AccountMeta::new(balance_pda, false),
            AccountMeta::new_readonly(vault_config_pda, false),
        ],
        data: VaultInstruction::RelayerSpotWithdraw {
            user_wallet: *user_wallet,
            token_index,
            amount,
            account_index: 0,
            amount_e6: amount as i64,
        }
        .try_to_vec()
        .unwrap(),
    }
}

// ============================================================
// Test: Deposit + Auto-Init
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

    assert!(read_spot_balance(&mut banks_client, &balance_pda).await.is_none());

    let ix = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &user, token_index, 1000_000_000);
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], recent_blockhash);
    banks_client.process_transaction(tx).await.unwrap();

    let balance = read_spot_balance(&mut banks_client, &balance_pda).await.unwrap();
    assert_eq!(balance.discriminator, SPOT_TOKEN_BALANCE_DISCRIMINATOR);
    assert_eq!(balance.wallet, user);
    assert_eq!(balance.token_index, token_index);
    assert_eq!(balance.available_e6, 1000_000_000);
    assert_eq!(balance.locked_e6, 0);

    let ix2 = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &user, token_index, 500_000_000);
    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    let tx2 = Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], recent_blockhash);
    banks_client.process_transaction(tx2).await.unwrap();

    let balance2 = read_spot_balance(&mut banks_client, &balance_pda).await.unwrap();
    assert_eq!(balance2.available_e6, 1500_000_000); // 1000 + 500
}

// ============================================================
// Test: Withdraw success + insufficient
// ============================================================
#[tokio::test]
async fn test_relayer_spot_withdraw() {
    let program_id = Pubkey::new_unique();
    let program_test = setup_vault(&program_id).await;
    let (mut banks_client, payer, _) = program_test.start().await;

    initialize_vault_config(&mut banks_client, &payer, &program_id).await;

    let user = Pubkey::new_unique();
    let token_index: u16 = 1;

    let ix = build_relayer_spot_deposit_ix(&program_id, &payer.pubkey(), &user, token_index, 1000_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let ix2 = build_relayer_spot_withdraw_ix(&program_id, &payer.pubkey(), &user, token_index, 400_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix2], Some(&payer.pubkey()), &[&payer], bh)).await.unwrap();

    let (balance_pda, _) = derive_balance_pda(&program_id, &user, token_index);
    let balance = read_spot_balance(&mut banks_client, &balance_pda).await.unwrap();
    assert_eq!(balance.available_e6, 600_000_000); // 1000 - 400

    let ix3 = build_relayer_spot_withdraw_ix(&program_id, &payer.pubkey(), &user, token_index, 700_000_000);
    let bh = banks_client.get_latest_blockhash().await.unwrap();
    let result = banks_client.process_transaction(Transaction::new_signed_with_payer(&[ix3], Some(&payer.pubkey()), &[&payer], bh)).await;
    assert!(result.is_err()); // InsufficientBalance
}
