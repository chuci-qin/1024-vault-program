//! Vault Program Integration Tests

use borsh::{BorshSerialize, BorshDeserialize};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_instruction,
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

// Note: test_initialize 暂时跳过，因为涉及复杂的PDA和Token Account创建
// 将在完整的端到端测试中验证
// #[tokio::test]
// async fn test_initialize() { ... }

#[tokio::test]
async fn test_initialize_user() {
    let program_id = Pubkey::new_unique();
    let mut program_test = ProgramTest::new(
        "vault_program",
        program_id,
        processor!(vault_program::processor::process_instruction),
    );

    let (mut banks_client, payer, recent_blockhash) = program_test.start().await;

    let user = Keypair::new();
    
    // 给用户账户空投SOL
    let airdrop_ix = system_instruction::transfer(
        &payer.pubkey(),
        &user.pubkey(),
        1_000_000_000, // 1 SOL
    );
    
    let airdrop_tx = Transaction::new_signed_with_payer(
        &[airdrop_ix],
        Some(&payer.pubkey()),
        &[&payer],
        recent_blockhash,
    );
    
    banks_client.process_transaction(airdrop_tx).await.unwrap();

    // 派生UserAccount PDA
    let (user_account_pda, _bump) = Pubkey::find_program_address(
        &[b"user", user.pubkey().as_ref()],
        &program_id,
    );

    // 创建InitializeUser指令
    let instruction_data = VaultInstruction::InitializeUser;

    let instruction = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user.pubkey(), true),
            AccountMeta::new(user_account_pda, false),
            AccountMeta::new_readonly(solana_program::system_program::id(), false),
        ],
        data: instruction_data.try_to_vec().unwrap(),
    };

    let recent_blockhash = banks_client.get_latest_blockhash().await.unwrap();
    
    let transaction = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&user.pubkey()),
        &[&user],
        recent_blockhash,
    );

    // 执行交易
    banks_client.process_transaction(transaction).await.unwrap();

    // 验证UserAccount
    let user_account_data = banks_client
        .get_account(user_account_pda)
        .await
        .unwrap()
        .unwrap();

    let user_account = UserAccount::try_from_slice(&user_account_data.data).unwrap();
    assert_eq!(user_account.wallet, user.pubkey());
    assert_eq!(user_account.available_balance_e6, 0);
    assert_eq!(user_account.locked_margin_e6, 0);
}

#[tokio::test]
async fn test_state_calculations() {
    // 测试 UserAccount 的 equity 计算
    let user_account = UserAccount {
        discriminator: UserAccount::DISCRIMINATOR,
        wallet: Pubkey::new_unique(),
        bump: 255,
        available_balance_e6: 1000_000_000, // 1000 USDC
        locked_margin_e6: 500_000_000,      // 500 USDC
        unrealized_pnl_e6: 200_000_000,     // 200 USDC
        total_deposited_e6: 1000_000_000,
        total_withdrawn_e6: 0,
        last_update_ts: 0,
        reserved: [0; 64],
    };

    // equity = available + locked + unrealized_pnl
    // = 1000 + 500 + 200 = 1700 USDC
    assert_eq!(user_account.equity(), 1700_000_000);
    
    // 验证结构体字段
    assert_eq!(user_account.available_balance_e6, 1000_000_000);
    assert_eq!(user_account.locked_margin_e6, 500_000_000);
    assert_eq!(user_account.unrealized_pnl_e6, 200_000_000);
}

// 注意: InsuranceFund 相关测试已移动到 1024-fund-program
// 参见: onchain-program/1024-fund-program/tests/insurance_fund_test.rs

