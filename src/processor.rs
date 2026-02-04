//! Vault Program Processor
//!
//! Vault Program 职责: 纯用户资金托管 (用户的钱)
//! 
//! 架构原则:
//! - Vault Program = 用户资金托管 (入金/出金/保证金)
//! - Fund Program = 资金池管理 (保险基金/手续费/返佣等)
//!
//! 详见: onchain-program/vault_vs_fund.md

use crate::{
    error::VaultError,
    instruction::VaultInstruction,
    state::*,
    token_compat,
    utils::*,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::{invoke, invoke_signed},
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};

// ============================================================================
// PM Fee Config 字段偏移量 (基于 Fund Program state.rs)
// ============================================================================
mod pm_fee_config_offsets {
    pub const _DISCRIMINATOR: usize = 0;      // 8 bytes
    pub const FEE_VAULT: usize = 8;           // 32 bytes (Pubkey)
    pub const _BUMP: usize = 40;              // 1 byte
    pub const MINTING_FEE_BPS: usize = 41;    // 2 bytes (u16)
    pub const REDEMPTION_FEE_BPS: usize = 43; // 2 bytes (u16)
    pub const _TRADING_FEE_TAKER_BPS: usize = 45; // 2 bytes (u16)
    pub const _TRADING_FEE_MAKER_BPS: usize = 47; // 2 bytes (u16)
    pub const _SETTLEMENT_FEE_BPS: usize = 49;    // 2 bytes (u16)
    pub const _PROTOCOL_SHARE_BPS: usize = 51;    // 2 bytes (u16)
    pub const _MAKER_REWARD_SHARE_BPS: usize = 53; // 2 bytes (u16)
    pub const _CREATOR_SHARE_BPS: usize = 55;     // 2 bytes (u16)
    pub const TOTAL_MINTING_FEE: usize = 57;      // 8 bytes (i64)
    pub const TOTAL_REDEMPTION_FEE: usize = 65;   // 8 bytes (i64)
    pub const MIN_SIZE: usize = 150;
}

/// 辅助函数：反序列化账户数据
fn deserialize_account<T: BorshDeserialize>(data: &[u8]) -> Result<T, std::io::Error> {
    let mut slice = data;
    T::deserialize(&mut slice)
}

/// 验证 CPI 调用方是否授权
fn verify_cpi_caller(
    vault_config: &VaultConfig,
    caller_program: &AccountInfo,
) -> ProgramResult {
    if !vault_config.is_authorized_caller(caller_program.key) {
        msg!("CPI caller {} not authorized", caller_program.key);
        return Err(VaultError::UnauthorizedCaller.into());
    }
    
    // 验证是已知的授权调用方
    let (expected_ledger_config, _bump) = Pubkey::find_program_address(
        &[b"ledger_config"],
        &vault_config.ledger_program
    );
    
    if caller_program.key == &expected_ledger_config {
        msg!("✅ CPI caller verified as ledger_config PDA");
    } else if caller_program.key == &vault_config.ledger_program {
        msg!("✅ CPI caller is ledger_program");
    } else if vault_config.authorized_callers.iter().any(|pk| pk == caller_program.key && *pk != Pubkey::default()) {
        msg!("✅ CPI caller in authorized list");
    } else if vault_config.fund_program != Pubkey::default() && caller_program.key == &vault_config.fund_program {
        msg!("✅ CPI caller is fund_program");
    } else {
        msg!("❌ Unknown CPI caller: {}", caller_program.key);
        return Err(VaultError::InvalidCallerPda.into());
    }
    
    Ok(())
}

/// Program state handler
pub struct Processor;

impl Processor {
    /// Process instruction
    pub fn process(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> ProgramResult {
        let instruction = VaultInstruction::try_from_slice(instruction_data)?;

        match instruction {
            VaultInstruction::Initialize {
                ledger_program,
                delegation_program,
                fund_program,
            } => {
                msg!("Instruction: Initialize");
                Self::process_initialize(
                    program_id,
                    accounts,
                    ledger_program,
                    delegation_program,
                    fund_program,
                )
            }
            VaultInstruction::InitializeUser => {
                msg!("Instruction: InitializeUser");
                Self::process_initialize_user(program_id, accounts)
            }
            VaultInstruction::Deposit { amount } => {
                msg!("Instruction: Deposit");
                Self::process_deposit(accounts, amount)
            }
            VaultInstruction::Withdraw { amount } => {
                msg!("Instruction: Withdraw");
                Self::process_withdraw(accounts, amount)
            }
            VaultInstruction::LockMargin { amount } => {
                msg!("Instruction: LockMargin");
                Self::process_lock_margin(accounts, amount)
            }
            VaultInstruction::ReleaseMargin { amount } => {
                msg!("Instruction: ReleaseMargin");
                Self::process_release_margin(accounts, amount)
            }
            VaultInstruction::ClosePositionSettle {
                margin_to_release,
                realized_pnl,
                fee,
            } => {
                msg!("Instruction: ClosePositionSettle");
                Self::process_close_position_settle(accounts, margin_to_release, realized_pnl, fee)
            }
            VaultInstruction::LiquidatePosition {
                margin,
                user_remainder,
                liquidation_penalty,
            } => {
                msg!("Instruction: LiquidatePosition");
                Self::process_liquidate_position(program_id, accounts, margin, user_remainder, liquidation_penalty)
            }
            VaultInstruction::AddAuthorizedCaller { caller } => {
                msg!("Instruction: AddAuthorizedCaller");
                Self::process_add_authorized_caller(accounts, caller)
            }
            VaultInstruction::RemoveAuthorizedCaller { caller } => {
                msg!("Instruction: RemoveAuthorizedCaller");
                Self::process_remove_authorized_caller(accounts, caller)
            }
            VaultInstruction::SetPaused { paused } => {
                msg!("Instruction: SetPaused");
                Self::process_set_paused(accounts, paused)
            }
            VaultInstruction::UpdateAdmin { new_admin } => {
                msg!("Instruction: UpdateAdmin");
                Self::process_update_admin(accounts, new_admin)
            }
            VaultInstruction::SetFundProgram { fund_program } => {
                msg!("Instruction: SetFundProgram");
                Self::process_set_fund_program(accounts, fund_program)
            }
            VaultInstruction::SetLedgerProgram { ledger_program } => {
                msg!("Instruction: SetLedgerProgram");
                Self::process_set_ledger_program(accounts, ledger_program)
            }
            VaultInstruction::AdminForceReleaseMargin { amount } => {
                msg!("Instruction: AdminForceReleaseMargin");
                Self::process_admin_force_release_margin(accounts, amount)
            }
            
            // Prediction Market 指令
            VaultInstruction::InitializePredictionMarketUser => {
                msg!("Instruction: InitializePredictionMarketUser");
                Self::process_initialize_prediction_market_user(program_id, accounts)
            }
            VaultInstruction::PredictionMarketLock { amount } => {
                msg!("Instruction: PredictionMarketLock");
                Self::process_prediction_market_lock(accounts, amount)
            }
            VaultInstruction::PredictionMarketUnlock { amount } => {
                msg!("Instruction: PredictionMarketUnlock");
                Self::process_prediction_market_unlock(accounts, amount)
            }
            VaultInstruction::PredictionMarketSettle { locked_amount, settlement_amount } => {
                msg!("Instruction: PredictionMarketSettle");
                Self::process_prediction_market_settle(program_id, accounts, locked_amount, settlement_amount)
            }
            VaultInstruction::PredictionMarketClaimSettlement => {
                msg!("Instruction: PredictionMarketClaimSettlement");
                Self::process_prediction_market_claim_settlement(accounts)
            }
            VaultInstruction::AdminPredictionMarketForceUnlock { amount } => {
                msg!("Instruction: AdminPredictionMarketForceUnlock");
                Self::process_admin_prediction_market_force_unlock(accounts, amount)
            }
            VaultInstruction::PredictionMarketLockWithFee { gross_amount } => {
                msg!("Instruction: PredictionMarketLockWithFee");
                Self::process_prediction_market_lock_with_fee(program_id, accounts, gross_amount)
            }
            VaultInstruction::PredictionMarketUnlockWithFee { gross_amount } => {
                msg!("Instruction: PredictionMarketUnlockWithFee");
                Self::process_prediction_market_unlock_with_fee(program_id, accounts, gross_amount)
            }
            VaultInstruction::PredictionMarketTradeWithFee { trade_amount, is_taker } => {
                msg!("Instruction: PredictionMarketTradeWithFee");
                Self::process_prediction_market_trade_with_fee(program_id, accounts, trade_amount, is_taker)
            }
            VaultInstruction::PredictionMarketSettleWithFee { locked_amount, settlement_amount } => {
                msg!("Instruction: PredictionMarketSettleWithFee");
                Self::process_prediction_market_settle_with_fee(program_id, accounts, locked_amount, settlement_amount)
            }
            
            // Relayer 指令
            VaultInstruction::RelayerDeposit { user_wallet, amount } => {
                msg!("Instruction: RelayerDeposit");
                Self::process_relayer_deposit(program_id, accounts, user_wallet, amount)
            }
            VaultInstruction::RelayerWithdraw { user_wallet, amount } => {
                msg!("Instruction: RelayerWithdraw");
                Self::process_relayer_withdraw(program_id, accounts, user_wallet, amount)
            }
            
            // Spot 交易指令
            VaultInstruction::InitializeSpotUser => {
                msg!("Instruction: InitializeSpotUser");
                Self::process_initialize_spot_user(program_id, accounts)
            }
            VaultInstruction::SpotDeposit { token_index, amount } => {
                msg!("Instruction: SpotDeposit");
                Self::process_spot_deposit(program_id, accounts, token_index, amount)
            }
            VaultInstruction::SpotWithdraw { token_index, amount } => {
                msg!("Instruction: SpotWithdraw");
                Self::process_spot_withdraw(program_id, accounts, token_index, amount)
            }
            VaultInstruction::SpotLockBalance { token_index, amount } => {
                msg!("Instruction: SpotLockBalance");
                Self::process_spot_lock_balance(accounts, token_index, amount)
            }
            VaultInstruction::SpotUnlockBalance { token_index, amount } => {
                msg!("Instruction: SpotUnlockBalance");
                Self::process_spot_unlock_balance(accounts, token_index, amount)
            }
            VaultInstruction::SpotSettleTrade { is_buy, base_token_index, quote_token_index, base_amount, quote_amount, sequence } => {
                msg!("Instruction: SpotSettleTrade");
                Self::process_spot_settle_trade(accounts, is_buy, base_token_index, quote_token_index, base_amount, quote_amount, sequence)
            }
            VaultInstruction::RelayerSpotDeposit { user_wallet, token_index, amount } => {
                msg!("Instruction: RelayerSpotDeposit");
                Self::process_relayer_spot_deposit(program_id, accounts, user_wallet, token_index, amount)
            }
            VaultInstruction::RelayerSpotWithdraw { user_wallet, token_index, amount } => {
                msg!("Instruction: RelayerSpotWithdraw");
                Self::process_relayer_spot_withdraw(program_id, accounts, user_wallet, token_index, amount)
            }
            
            // Spot 统一账户指令 (2025-12-31 新增)
            VaultInstruction::RelayerSpotSettleTrade { 
                maker_wallet, taker_wallet, base_token_index, quote_token_index,
                base_amount_e6, quote_amount_e6, maker_fee_e6, taker_fee_e6,
                taker_is_buy, sequence 
            } => {
                msg!("Instruction: RelayerSpotSettleTrade");
                Self::process_relayer_spot_settle_trade(
                    program_id, accounts, maker_wallet, taker_wallet,
                    base_token_index, quote_token_index, base_amount_e6, quote_amount_e6,
                    maker_fee_e6, taker_fee_e6, taker_is_buy, sequence
                )
            }
            VaultInstruction::SpotAllocateFromVault { user_wallet, amount } => {
                msg!("Instruction: SpotAllocateFromVault");
                Self::process_spot_allocate_from_vault(program_id, accounts, user_wallet, amount)
            }
            VaultInstruction::SpotReleaseToVault { user_wallet, amount } => {
                msg!("Instruction: SpotReleaseToVault");
                Self::process_spot_release_to_vault(program_id, accounts, user_wallet, amount)
            }

            // =========================================================================
            // 站内支付相关指令
            // =========================================================================

            VaultInstruction::RelayerInternalTransfer {
                from_wallet,
                to_wallet,
                amount,
                fee,
                transfer_type,
                reference_hash,
            } => {
                msg!("Instruction: RelayerInternalTransfer");
                Self::process_relayer_internal_transfer(
                    program_id, accounts, from_wallet, to_wallet, amount, fee, transfer_type, reference_hash
                )
            }
            VaultInstruction::InitRecurringAuth {
                payer,
                payee,
                amount,
                interval_seconds,
                max_cycles,
                registration_fee,
            } => {
                msg!("Instruction: InitRecurringAuth");
                Self::process_init_recurring_auth(
                    program_id, accounts, payer, payee, amount, interval_seconds, max_cycles, registration_fee
                )
            }
            VaultInstruction::ExecuteRecurringPayment {
                payer,
                payee,
                amount,
                fee,
                cycle_count,
            } => {
                msg!("Instruction: ExecuteRecurringPayment");
                Self::process_execute_recurring_payment(
                    program_id, accounts, payer, payee, amount, fee, cycle_count
                )
            }
            VaultInstruction::CancelRecurringAuth { payer, payee } => {
                msg!("Instruction: CancelRecurringAuth");
                Self::process_cancel_recurring_auth(program_id, accounts, payer, payee)
            }
        }
    }

    /// 处理初始化
    fn process_initialize(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        ledger_program: Pubkey,
        delegation_program: Pubkey,
        fund_program: Pubkey,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let usdc_mint = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let _system_program = next_account_info(account_info_iter)?;

        // 验证admin签名
        assert_signer(admin)?;

        // 创建VaultConfig PDA
        let (vault_config_pda, vault_config_bump) =
            Pubkey::find_program_address(&[b"vault_config"], program_id);

        if vault_config_info.key != &vault_config_pda {
            return Err(VaultError::InvalidPda.into());
        }

        // 创建账户
        let rent = Rent::get()?;
        let space = VAULT_CONFIG_SIZE;
        let lamports = rent.minimum_balance(space);

        invoke_signed(
            &system_instruction::create_account(
                admin.key,
                vault_config_info.key,
                lamports,
                space as u64,
                program_id,
            ),
            &[admin.clone(), vault_config_info.clone()],
            &[&[b"vault_config", &[vault_config_bump]]],
        )?;

        // 初始化数据
        let vault_config = VaultConfig {
            discriminator: VaultConfig::DISCRIMINATOR,
            admin: *admin.key,
            usdc_mint: *usdc_mint.key,
            vault_token_account: *vault_token_account.key,
            authorized_callers: [Pubkey::default(); 10], // 固定大小数组
            ledger_program,
            fund_program, // 不再是 Option
            delegation_program,
            total_deposits: 0,
            total_locked: 0,
            is_paused: false,
            reserved: [0u8; 32],
        };

        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Vault initialized");
        msg!("Ledger Program: {}", ledger_program);
        msg!("Fund Program: {}", fund_program);
        msg!("Delegation Program: {}", delegation_program);
        Ok(())
    }

    /// 处理初始化用户账户
    fn process_initialize_user(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let _system_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        let (user_account_pda, bump) = Pubkey::find_program_address(&[b"user", user.key.as_ref()], program_id);

        if user_account_info.key != &user_account_pda {
            return Err(VaultError::InvalidPda.into());
        }

        let rent = Rent::get()?;
        let space = USER_ACCOUNT_SIZE;
        let lamports = rent.minimum_balance(space);

        invoke_signed(
            &system_instruction::create_account(
                user.key,
                user_account_info.key,
                lamports,
                space as u64,
                program_id,
            ),
            &[user.clone(), user_account_info.clone()],
            &[&[b"user", user.key.as_ref(), &[bump]]],
        )?;

        let user_account = UserAccount {
            discriminator: UserAccount::DISCRIMINATOR,
            wallet: *user.key,
            bump,
            available_balance_e6: 0,
            locked_margin_e6: 0,
            unrealized_pnl_e6: 0,
            total_deposited_e6: 0,
            total_withdrawn_e6: 0,
            last_update_ts: 0,
            reserved: [0; 64],
        };

        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("User account initialized for {}", user.key);
        Ok(())
    }

    /// 处理入金
    fn process_deposit(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;
        assert_writable(user_account_info)?;
        assert_writable(vault_config_info)?;

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.is_paused {
            return Err(VaultError::VaultPaused.into());
        }

        // SPL Token Transfer (用户 → Vault) - 使用 token_compat 支持 Token-2022
        token_compat::transfer(
            token_program,
            user_token_account,
            vault_token_account,
            user,
            amount,
            None, // 用户签名，不需要 PDA seeds
        )?;

        // 更新UserAccount
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, amount as i64)?;
        user_account.total_deposited_e6 = checked_add(user_account.total_deposited_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        // 更新VaultConfig
        vault_config.total_deposits = checked_add_u64(vault_config.total_deposits, amount)?;
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Deposited {} e6 for {}", amount, user.key);
        Ok(())
    }

    /// 处理出金
    fn process_withdraw(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;
        assert_writable(user_account_info)?;

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.is_paused {
            return Err(VaultError::VaultPaused.into());
        }

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        if user_account.available_balance_e6 < amount as i64 {
            return Err(VaultError::InsufficientBalance.into());
        }

        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, amount as i64)?;
        user_account.total_withdrawn_e6 = checked_add(user_account.total_withdrawn_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        // SPL Token Transfer (Vault → 用户) - 使用 token_compat 支持 Token-2022
        let (_vault_config_pda, vault_config_bump) =
            Pubkey::find_program_address(&[b"vault_config"], vault_config_info.owner);

        token_compat::transfer(
            token_program,
            vault_token_account,
            user_token_account,
            vault_config_info,
            amount,
            Some(&[b"vault_config", &[vault_config_bump]]),
        )?;

        msg!("Withdrawn {} e6 for {}", amount, user.key);
        Ok(())
    }

    /// 处理锁定保证金 (CPI only)
    fn process_lock_margin(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        if user_account.available_balance_e6 < amount as i64 {
            return Err(VaultError::InsufficientBalance.into());
        }

        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, amount as i64)?;
        user_account.locked_margin_e6 = checked_add(user_account.locked_margin_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("Locked margin: {} e6 for {}", amount, user_account.wallet);
        Ok(())
    }

    /// 处理释放保证金 (CPI only)
    fn process_release_margin(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        if user_account.locked_margin_e6 < amount as i64 {
            return Err(VaultError::InsufficientMargin.into());
        }

        user_account.locked_margin_e6 = checked_sub(user_account.locked_margin_e6, amount as i64)?;
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("Released margin: {} e6 for {}", amount, user_account.wallet);
        Ok(())
    }

    /// 处理平仓结算 (CPI only)
    /// 
    /// 注意: 手续费的分配 (到保险基金/返佣等) 由 Ledger Program 
    /// 单独通过 CPI 调用 Fund Program 处理
    fn process_close_position_settle(
        accounts: &[AccountInfo],
        margin_to_release: u64,
        realized_pnl: i64,
        fee: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        // 1. 释放保证金
        if user_account.locked_margin_e6 < margin_to_release as i64 {
            return Err(VaultError::InsufficientMargin.into());
        }
        user_account.locked_margin_e6 = checked_sub(user_account.locked_margin_e6, margin_to_release as i64)?;
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, margin_to_release as i64)?;
        
        // 🔧 自动清理残留 locked_margin
        // 当释放后 locked_margin 小于 1 USDC (1_000_000 e6) 时，自动释放全部剩余
        // 这解决了精度累积误差导致的残留问题
        if user_account.locked_margin_e6 > 0 && user_account.locked_margin_e6 < 1_000_000 {
            msg!("🔧 Auto-cleanup: releasing residual locked_margin={}", user_account.locked_margin_e6);
            user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, user_account.locked_margin_e6)?;
            user_account.locked_margin_e6 = 0;
        }

        // 2. 结算盈亏
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, realized_pnl)?;

        // 3. 扣除手续费 (手续费的分配由 Ledger 调用 Fund Program)
        if user_account.available_balance_e6 < fee as i64 {
            return Err(VaultError::InsufficientBalance.into());
        }
        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, fee as i64)?;

        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!(
            "ClosePositionSettle: margin={}, pnl={}, fee={}",
            margin_to_release,
            realized_pnl,
            fee
        );
        Ok(())
    }

    /// 处理清算 (CPI only)
    /// 
    /// 执行清算时的完整资金处理:
    /// 1. 清空用户锁定保证金
    /// 2. 返还剩余给用户
    /// 3. 将清算罚金从 Vault Token Account 转入 Insurance Fund Vault
    fn process_liquidate_position(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        _margin: u64,
        user_remainder: u64,
        liquidation_penalty: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let insurance_fund_vault = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;
        assert_writable(vault_token_account)?;
        assert_writable(insurance_fund_vault)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        // 1. 清空锁定保证金
        user_account.locked_margin_e6 = 0;
        
        // 2. 返还剩余给用户 (如果有)
        if user_remainder > 0 {
            user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, user_remainder as i64)?;
        }

        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        // 3. 将清算罚金从 Vault Token Account 转入 Insurance Fund Vault
        if liquidation_penalty > 0 {
            // 验证 vault_token_account 是 VaultConfig 中配置的
            if vault_config.vault_token_account != *vault_token_account.key {
                msg!("❌ Invalid vault token account");
                return Err(VaultError::InvalidAccount.into());
            }
            
            // 使用 VaultConfig PDA 作为 authority 签名
            let (vault_config_pda, bump) = Pubkey::find_program_address(
                &[b"vault_config"],
                program_id,
            );
            
            if vault_config_pda != *vault_config_info.key {
                msg!("❌ VaultConfig PDA mismatch");
                return Err(VaultError::InvalidAccount.into());
            }
            
            // 使用 token_compat 支持 Token-2022 (USDC)
            token_compat::transfer(
                token_program,
                vault_token_account,
                insurance_fund_vault,
                vault_config_info,
                liquidation_penalty,
                Some(&[b"vault_config", &[bump]]),
            )?;
            
            msg!(
                "✅ Liquidation penalty {} transferred to Insurance Fund",
                liquidation_penalty
            );
        }

        msg!(
            "Liquidated user account: remainder={}, penalty={}",
            user_remainder,
            liquidation_penalty
        );
        Ok(())
    }

    fn process_add_authorized_caller(accounts: &[AccountInfo], caller: Pubkey) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.admin != *admin.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        // 检查是否已存在
        let already_exists = vault_config.authorized_callers.iter().any(|pk| *pk == caller);
        if already_exists {
            msg!("Caller already authorized: {}", caller);
            return Ok(());
        }

        // 找到一个空槽位并添加
        let mut added = false;
        for slot in vault_config.authorized_callers.iter_mut() {
            if *slot == Pubkey::default() {
                *slot = caller;
                added = true;
                break;
            }
        }

        if added {
            vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;
            msg!("Added authorized caller: {}", caller);
        } else {
            msg!("❌ No empty slot available for authorized caller");
            return Err(VaultError::InvalidAccount.into());
        }

        Ok(())
    }

    fn process_remove_authorized_caller(accounts: &[AccountInfo], caller: Pubkey) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.admin != *admin.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        // 找到并移除 caller (设为默认值)
        let mut removed = false;
        for slot in vault_config.authorized_callers.iter_mut() {
            if *slot == caller {
                *slot = Pubkey::default();
                removed = true;
                break;
            }
        }

        if removed {
            vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;
            msg!("Removed authorized caller: {}", caller);
        } else {
            msg!("Caller not found in authorized list: {}", caller);
        }

        Ok(())
    }

    fn process_set_paused(accounts: &[AccountInfo], paused: bool) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.admin != *admin.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        vault_config.is_paused = paused;
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Vault {}", if paused { "paused" } else { "resumed" });
        Ok(())
    }

    fn process_update_admin(accounts: &[AccountInfo], new_admin: Pubkey) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let current_admin = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(current_admin)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.admin != *current_admin.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        vault_config.admin = new_admin;
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Admin updated to: {}", new_admin);
        Ok(())
    }
    
    fn process_set_fund_program(accounts: &[AccountInfo], fund_program: Pubkey) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.admin != *admin.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        vault_config.fund_program = fund_program;
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Fund program set to: {}", fund_program);
        Ok(())
    }
    
    fn process_set_ledger_program(accounts: &[AccountInfo], ledger_program: Pubkey) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.admin != *admin.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        vault_config.ledger_program = ledger_program;
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Ledger program set to: {}", ledger_program);
        Ok(())
    }

    /// Admin 强制释放用户锁定保证金
    /// 
    /// 用于处理用户没有任何持仓但 locked_margin 残留的异常情况
    fn process_admin_force_release_margin(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        // 验证 admin 签名
        assert_signer(admin)?;
        assert_writable(user_account_info)?;

        // 验证 admin 权限
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        // 计算要释放的金额
        let release_amount = if amount == 0 {
            // 释放全部 locked_margin
            user_account.locked_margin_e6
        } else {
            amount as i64
        };

        // 验证释放金额不超过 locked_margin
        if release_amount > user_account.locked_margin_e6 {
            return Err(VaultError::InsufficientMargin.into());
        }

        if release_amount <= 0 {
            msg!("No locked margin to release");
            return Ok(());
        }

        // 释放保证金：locked -> available
        user_account.locked_margin_e6 = checked_sub(user_account.locked_margin_e6, release_amount)?;
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, release_amount)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!(
            "Admin force released {} e6 locked margin for user {}. New locked: {}, available: {}",
            release_amount,
            user_account.wallet,
            user_account.locked_margin_e6,
            user_account.available_balance_e6
        );
        
        Ok(())
    }

    // =========================================================================
    // Prediction Market 指令实现
    // =========================================================================

    /// 初始化预测市场用户账户
    fn process_initialize_prediction_market_user(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let _system_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        let (pm_user_pda, bump) = Pubkey::find_program_address(
            &[PREDICTION_MARKET_USER_SEED, user.key.as_ref()],
            program_id
        );

        if pm_user_account_info.key != &pm_user_pda {
            return Err(VaultError::InvalidPda.into());
        }

        let rent = Rent::get()?;
        let space = PREDICTION_MARKET_USER_ACCOUNT_SIZE;
        let lamports = rent.minimum_balance(space);

        invoke_signed(
            &system_instruction::create_account(
                user.key,
                pm_user_account_info.key,
                lamports,
                space as u64,
                program_id,
            ),
            &[user.clone(), pm_user_account_info.clone()],
            &[&[PREDICTION_MARKET_USER_SEED, user.key.as_ref(), &[bump]]],
        )?;

        let pm_user_account = PredictionMarketUserAccount::new(
            *user.key,
            bump,
            solana_program::clock::Clock::get()?.unix_timestamp,
        );
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        msg!("Prediction market user account initialized for {}", user.key);
        Ok(())
    }

    /// 预测市场锁定 (CPI only)
    /// 
    /// 如果 PMUserAccount 不存在，会自动创建（需要额外的 payer 和 system_program 账户）
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[writable]` PMUserAccount PDA
    /// 3. `[]` Caller Program
    /// 4. `[signer, writable]` Payer (optional, for auto-init)
    /// 5. `[]` System Program (optional, for auto-init)
    fn process_prediction_market_lock(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;
        
        // Optional accounts for auto-init
        let payer_info = next_account_info(account_info_iter).ok();
        let system_program_info = next_account_info(account_info_iter).ok();

        assert_writable(user_account_info)?;
        assert_writable(pm_user_account_info)?;

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 从 UserAccount 扣除
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        if user_account.available_balance_e6 < amount as i64 {
            return Err(VaultError::InsufficientBalance.into());
        }
        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        // Auto-init PMUserAccount if empty
        if pm_user_account_info.data_is_empty() {
            msg!("Auto-initializing PMUserAccount for {}", user_account.wallet);
            
            let payer = payer_info.ok_or_else(|| {
                msg!("❌ PMUserAccount not initialized and no payer provided");
                VaultError::InvalidAccount
            })?;
            let system_program = system_program_info.ok_or_else(|| {
                msg!("❌ PMUserAccount not initialized and no system_program provided");
                VaultError::InvalidAccount
            })?;
            
            // Derive PDA to get bump
            let (pm_user_pda, bump) = Pubkey::find_program_address(
                &[PREDICTION_MARKET_USER_SEED, user_account.wallet.as_ref()],
                vault_config_info.owner, // Vault Program ID
            );
            
            if pm_user_account_info.key != &pm_user_pda {
                msg!("❌ Invalid PMUserAccount PDA");
                return Err(VaultError::InvalidPda.into());
            }
            
            let rent = Rent::get()?;
            let space = PREDICTION_MARKET_USER_ACCOUNT_SIZE;
            let lamports = rent.minimum_balance(space);
            
            // Create account with PDA seeds
            invoke_signed(
                &system_instruction::create_account(
                    payer.key,
                    pm_user_account_info.key,
                    lamports,
                    space as u64,
                    vault_config_info.owner, // Vault Program ID
                ),
                &[payer.clone(), pm_user_account_info.clone(), system_program.clone()],
                &[&[PREDICTION_MARKET_USER_SEED, user_account.wallet.as_ref(), &[bump]]],
            )?;
            
            let pm_user_account = PredictionMarketUserAccount::new(
                user_account.wallet,
                bump,
                solana_program::clock::Clock::get()?.unix_timestamp,
            );
            pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;
            msg!("✅ PMUserAccount auto-initialized for {}", user_account.wallet);
        }

        // 增加 PredictionMarketUserAccount
        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        pm_user_account.prediction_market_lock(amount as i64, solana_program::clock::Clock::get()?.unix_timestamp);
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        msg!("Prediction market locked {} e6 for {}", amount, user_account.wallet);
        Ok(())
    }

    /// 预测市场释放锁定 (CPI only)
    fn process_prediction_market_unlock(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;
        assert_writable(pm_user_account_info)?;

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 从 PredictionMarketUserAccount 扣除
        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        pm_user_account.prediction_market_unlock(amount as i64, solana_program::clock::Clock::get()?.unix_timestamp)
            .map_err(|_| VaultError::InsufficientMargin)?;
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        // 增加 UserAccount
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("Prediction market unlocked {} e6 for {}", amount, user_account.wallet);
        Ok(())
    }

    /// 预测市场结算 (CPI only)
    /// 
    /// 支持自动创建 PMUserAccount (传递可选的 payer, system_program, user_wallet)
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` PMUserAccount PDA (will be auto-created if empty)
    /// 2. `[]` Caller Program
    /// 3. `[signer, writable]` Payer (optional, for auto-init)
    /// 4. `[]` System Program (optional, for auto-init)  
    /// 5. `[]` User Wallet (optional, for PDA derivation)
    fn process_prediction_market_settle(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        locked_amount: u64,
        settlement_amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;
        
        // Optional accounts for auto-init
        let payer_info = next_account_info(account_info_iter).ok();
        let system_program_info = next_account_info(account_info_iter).ok();
        let user_wallet_info = next_account_info(account_info_iter).ok();

        assert_writable(pm_user_account_info)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // Auto-init PMUserAccount if empty
        if pm_user_account_info.data_is_empty() {
            msg!("🔧 PMUserAccount not found, attempting auto-init for settle");
            
            let payer = payer_info.ok_or_else(|| {
                msg!("❌ PMUserAccount not initialized and no payer provided");
                VaultError::InvalidAccount
            })?;
            let system_program = system_program_info.ok_or_else(|| {
                msg!("❌ PMUserAccount not initialized and no system_program provided");
                VaultError::InvalidAccount
            })?;
            let user_wallet = user_wallet_info.ok_or_else(|| {
                msg!("❌ PMUserAccount not initialized and no user_wallet provided");
                VaultError::InvalidAccount
            })?;
            
            // Derive PDA to get bump
            let (pm_user_pda, bump) = Pubkey::find_program_address(
                &[PREDICTION_MARKET_USER_SEED, user_wallet.key.as_ref()],
                program_id,
            );
            
            if pm_user_account_info.key != &pm_user_pda {
                msg!("❌ Invalid PMUserAccount PDA: expected {}, got {}", pm_user_pda, pm_user_account_info.key);
                return Err(VaultError::InvalidPda.into());
            }
            
            let rent = Rent::get()?;
            let space = PREDICTION_MARKET_USER_ACCOUNT_SIZE;
            let lamports = rent.minimum_balance(space);
            
            invoke_signed(
                &system_instruction::create_account(
                    payer.key,
                    pm_user_account_info.key,
                    lamports,
                    space as u64,
                    program_id,
                ),
                &[payer.clone(), pm_user_account_info.clone(), system_program.clone()],
                &[&[PREDICTION_MARKET_USER_SEED, user_wallet.key.as_ref(), &[bump]]],
            )?;
            
            let pm_user_account = PredictionMarketUserAccount::new(
                *user_wallet.key,
                bump,
                solana_program::clock::Clock::get()?.unix_timestamp,
            );
            pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;
            msg!("✅ PMUserAccount auto-initialized for settle: {}", user_wallet.key);
        }

        // 正常结算逻辑
        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        pm_user_account.prediction_market_settle(
            locked_amount as i64,
            settlement_amount as i64,
            solana_program::clock::Clock::get()?.unix_timestamp,
        ).map_err(|_| VaultError::InsufficientMargin)?;
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        msg!("Prediction market settled: locked={}, settlement={}", locked_amount, settlement_amount);
        Ok(())
    }

    /// 预测市场领取结算收益
    fn process_prediction_market_claim_settlement(accounts: &[AccountInfo]) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;

        assert_signer(user)?;
        assert_writable(user_account_info)?;
        assert_writable(pm_user_account_info)?;

        // 从 PredictionMarketUserAccount 领取
        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        if pm_user_account.wallet != *user.key {
            return Err(VaultError::InvalidAccount.into());
        }
        let claim_amount = pm_user_account.prediction_market_claim_settlement(
            solana_program::clock::Clock::get()?.unix_timestamp
        );
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        if claim_amount <= 0 {
            msg!("No pending settlement to claim");
            return Ok(());
        }

        // 增加到 UserAccount
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        if user_account.wallet != *user.key {
            return Err(VaultError::InvalidAccount.into());
        }
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, claim_amount)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("Claimed prediction market settlement: {} e6", claim_amount);
        Ok(())
    }

    /// Admin 强制释放预测市场锁定
    fn process_admin_prediction_market_force_unlock(
        accounts: &[AccountInfo],
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        assert_writable(user_account_info)?;
        assert_writable(pm_user_account_info)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        let release_amount = if amount == 0 {
            pm_user_account.prediction_market_locked_e6
        } else {
            amount as i64
        };

        if release_amount <= 0 {
            msg!("No locked amount to release");
            return Ok(());
        }

        if pm_user_account.prediction_market_locked_e6 < release_amount {
            return Err(VaultError::InsufficientMargin.into());
        }

        pm_user_account.prediction_market_locked_e6 -= release_amount;
        pm_user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, release_amount)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("Admin force unlocked {} e6 from prediction market for {}", release_amount, user_account.wallet);
        Ok(())
    }

    // =========================================================================
    // Relayer 指令实现
    // =========================================================================

    /// Relayer 代理入金
    /// 
    /// 功能：
    /// 1. 验证 Admin 签名
    /// 2. 如果 UserAccount 不存在，自动创建
    /// 3. 增加用户余额
    /// 
    /// 测试网特性：Admin 可自由给任何用户入金（凭证模式）
    fn process_relayer_deposit(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        // 1. 验证 admin 签名和账户可写
        assert_signer(admin)?;
        assert_writable(user_account_info)?;
        // VaultConfig 不需要写入 (不更新 total_deposits)

        // 2. 验证 admin 权限
        // 兼容旧版 VaultConfig：直接读取 admin 字段 (offset 8, 32 bytes)
        let vault_config_data = vault_config_info.data.borrow();
        if vault_config_data.len() < 40 {
            msg!("❌ Invalid VaultConfig data length: {}", vault_config_data.len());
            return Err(VaultError::InvalidAccount.into());
        }
        
        // VaultConfig 结构: discriminator (8) + admin (32) + ...
        let stored_admin = Pubkey::try_from(&vault_config_data[8..40])
            .map_err(|_| VaultError::InvalidAccount)?;
        
        if stored_admin != *admin.key {
            msg!("❌ Invalid relayer: {} (expected admin: {})", admin.key, stored_admin);
            return Err(VaultError::InvalidRelayer.into());
        }
        
        // 跳过 is_paused 检查 (兼容旧版结构)

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // 3. 验证 UserAccount PDA
        let (user_account_pda, bump) = Pubkey::find_program_address(
            &[b"user", user_wallet.as_ref()],
            program_id
        );
        if user_account_info.key != &user_account_pda {
            msg!("❌ Invalid UserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        // 4. 检查 UserAccount 是否存在，不存在则创建
        if user_account_info.data_is_empty() {
            msg!("Creating new UserAccount for {}", user_wallet);
            
            let rent = Rent::get()?;
            let space = USER_ACCOUNT_SIZE;
            let lamports = rent.minimum_balance(space);

            invoke_signed(
                &system_instruction::create_account(
                    admin.key,
                    user_account_info.key,
                    lamports,
                    space as u64,
                    program_id,
                ),
                &[admin.clone(), user_account_info.clone(), system_program.clone()],
                &[&[b"user", user_wallet.as_ref(), &[bump]]],
            )?;

            // 初始化新账户
            let user_account = UserAccount {
                discriminator: UserAccount::DISCRIMINATOR,
                wallet: user_wallet,
                bump,
                available_balance_e6: amount as i64,
                locked_margin_e6: 0,
                unrealized_pnl_e6: 0,
                total_deposited_e6: amount as i64,
                total_withdrawn_e6: 0,
                last_update_ts: solana_program::clock::Clock::get()?.unix_timestamp,
                reserved: [0; 64],
            };
            user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

            msg!("✅ Created UserAccount and deposited {} e6 for {}", amount, user_wallet);
        } else {
            // 5. 更新现有 UserAccount
            let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
            
            // 验证钱包地址匹配
            if user_account.wallet != user_wallet {
                msg!("❌ Wallet mismatch: expected {}, got {}", user_wallet, user_account.wallet);
                return Err(VaultError::InvalidAccount.into());
            }

            user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, amount as i64)?;
            user_account.total_deposited_e6 = checked_add(user_account.total_deposited_e6, amount as i64)?;
            user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
            user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

            msg!("✅ RelayerDeposit {} e6 for {} (total: {})", 
                amount, user_wallet, user_account.available_balance_e6);
        }

        // 注意: 跳过更新 VaultConfig.total_deposits (兼容旧版结构)
        // 这是测试网的简化实现

        Ok(())
    }

    /// Relayer 代理出金
    /// 
    /// 功能：
    /// 1. 验证 Admin 签名
    /// 2. 验证用户余额充足
    /// 3. 扣除用户余额
    /// 
    /// 注意：Relayer 负责在 Solana 主网/Arbitrum 给用户转账
    fn process_relayer_withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        // 1. 验证 admin 签名和账户可写
        assert_signer(admin)?;
        assert_writable(user_account_info)?;

        // 2. 验证 admin 权限
        // 兼容旧版 VaultConfig：直接读取 admin 字段 (offset 8, 32 bytes)
        let vault_config_data = vault_config_info.data.borrow();
        if vault_config_data.len() < 40 {
            msg!("❌ Invalid VaultConfig data length: {}", vault_config_data.len());
            return Err(VaultError::InvalidAccount.into());
        }
        
        // VaultConfig 结构: discriminator (8) + admin (32) + ...
        let stored_admin = Pubkey::try_from(&vault_config_data[8..40])
            .map_err(|_| VaultError::InvalidAccount)?;
        
        if stored_admin != *admin.key {
            msg!("❌ Invalid relayer: {} (expected admin: {})", admin.key, stored_admin);
            return Err(VaultError::InvalidRelayer.into());
        }
        
        // 跳过 is_paused 检查 (兼容旧版结构)

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // 3. 验证 UserAccount PDA
        let (user_account_pda, _bump) = Pubkey::find_program_address(
            &[b"user", user_wallet.as_ref()],
            program_id
        );
        if user_account_info.key != &user_account_pda {
            msg!("❌ Invalid UserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        // 4. 验证账户存在
        if user_account_info.data_is_empty() {
            msg!("❌ UserAccount does not exist for {}", user_wallet);
            return Err(VaultError::NotInitialized.into());
        }

        // 5. 扣除用户余额
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        // 验证钱包地址匹配
        if user_account.wallet != user_wallet {
            msg!("❌ Wallet mismatch: expected {}, got {}", user_wallet, user_account.wallet);
            return Err(VaultError::InvalidAccount.into());
        }

        // 验证余额充足
        if user_account.available_balance_e6 < amount as i64 {
            msg!("❌ Insufficient balance: {} < {}", user_account.available_balance_e6, amount);
            return Err(VaultError::InsufficientBalance.into());
        }

        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, amount as i64)?;
        user_account.total_withdrawn_e6 = checked_add(user_account.total_withdrawn_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("✅ RelayerWithdraw {} e6 for {} (remaining: {})", 
            amount, user_wallet, user_account.available_balance_e6);
        
        Ok(())
    }

    // =========================================================================
    // V2 Fee Architecture: 在 Vault 层面收取手续费
    // =========================================================================

    /// 预测市场锁定 USDC 并扣除手续费 (CPI only)
    /// 
    /// V2 Fee Architecture: 在 Vault 层面收取手续费
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[writable]` PredictionMarketUserAccount
    /// 3. `[]` Caller Program
    /// 4. `[writable]` Vault Token Account
    /// 5. `[writable]` PM Fee Vault
    /// 6. `[writable]` PM Fee Config PDA
    /// 7. `[]` Token Program
    /// 8. `[signer, writable]` Payer (optional, for auto-init)
    /// 9. `[]` System Program (optional, for auto-init)
    fn process_prediction_market_lock_with_fee(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        gross_amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        
        // 解析必需账户
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;
        let vault_token_account_info = next_account_info(account_info_iter)?;
        let pm_fee_vault_info = next_account_info(account_info_iter)?;
        let pm_fee_config_info = next_account_info(account_info_iter)?;
        let token_program_info = next_account_info(account_info_iter)?;
        
        // 可选账户 (用于 auto-init PMUserAccount)
        let payer_info = next_account_info(account_info_iter).ok();
        let system_program_info = next_account_info(account_info_iter).ok();

        assert_writable(user_account_info)?;
        assert_writable(pm_user_account_info)?;
        assert_writable(vault_token_account_info)?;
        assert_writable(pm_fee_vault_info)?;
        assert_writable(pm_fee_config_info)?;

        if gross_amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // 1. 验证 VaultConfig 和 CPI 调用方
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 2. 验证 Vault Token Account
        if vault_token_account_info.key != &vault_config.vault_token_account {
            msg!("❌ Invalid vault_token_account");
            return Err(VaultError::InvalidAccount.into());
        }

        // 3. 读取 PM Fee Config 获取费率
        let pm_fee_config_data = pm_fee_config_info.try_borrow_data()?;
        if pm_fee_config_data.len() < pm_fee_config_offsets::MIN_SIZE {
            msg!("❌ PM Fee Config not initialized");
            return Err(VaultError::InvalidAccount.into());
        }
        
        // 读取 minting fee bps (offset 41, 2 bytes)
        let minting_fee_bps = u16::from_le_bytes([
            pm_fee_config_data[pm_fee_config_offsets::MINTING_FEE_BPS],
            pm_fee_config_data[pm_fee_config_offsets::MINTING_FEE_BPS + 1],
        ]);
        
        // 读取 PM Fee Vault 地址 (offset 8, 32 bytes) 用于验证
        let expected_fee_vault = Pubkey::new_from_array(
            pm_fee_config_data[pm_fee_config_offsets::FEE_VAULT..pm_fee_config_offsets::FEE_VAULT + 32]
                .try_into()
                .unwrap()
        );
        
        if pm_fee_vault_info.key != &expected_fee_vault {
            msg!("❌ PM Fee Vault mismatch: expected {}, got {}", expected_fee_vault, pm_fee_vault_info.key);
            return Err(VaultError::InvalidAccount.into());
        }
        
        drop(pm_fee_config_data);

        // 4. 计算 fee 和 net_amount
        let fee_amount = ((gross_amount as u128) * (minting_fee_bps as u128) / 10000) as u64;
        let net_amount = gross_amount.saturating_sub(fee_amount);
        
        msg!("PM Lock with Fee: gross={}, fee_bps={}, fee={}, net={}", 
             gross_amount, minting_fee_bps, fee_amount, net_amount);

        // 5. 从 UserAccount 扣除 gross_amount
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        if user_account.available_balance_e6 < gross_amount as i64 {
            msg!("❌ Insufficient balance: {} < {}", user_account.available_balance_e6, gross_amount);
            return Err(VaultError::InsufficientBalance.into());
        }
        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, gross_amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        // 6. Auto-init PMUserAccount if empty
        if pm_user_account_info.data_is_empty() {
            msg!("Auto-initializing PMUserAccount for {}", user_account.wallet);
            
            let payer = payer_info.ok_or_else(|| {
                msg!("❌ PMUserAccount not initialized and no payer provided");
                VaultError::InvalidAccount
            })?;
            let system_program = system_program_info.ok_or_else(|| {
                msg!("❌ PMUserAccount not initialized and no system_program provided");
                VaultError::InvalidAccount
            })?;
            
            let (pm_user_pda, bump) = Pubkey::find_program_address(
                &[PREDICTION_MARKET_USER_SEED, user_account.wallet.as_ref()],
                program_id,
            );
            
            if pm_user_account_info.key != &pm_user_pda {
                msg!("❌ Invalid PMUserAccount PDA");
                return Err(VaultError::InvalidPda.into());
            }
            
            let rent = Rent::get()?;
            let space = PREDICTION_MARKET_USER_ACCOUNT_SIZE;
            let lamports = rent.minimum_balance(space);
            
            invoke_signed(
                &system_instruction::create_account(
                    payer.key,
                    pm_user_account_info.key,
                    lamports,
                    space as u64,
                    program_id,
                ),
                &[payer.clone(), pm_user_account_info.clone(), system_program.clone()],
                &[&[PREDICTION_MARKET_USER_SEED, user_account.wallet.as_ref(), &[bump]]],
            )?;
            
            let pm_user_account = PredictionMarketUserAccount::new(
                user_account.wallet,
                bump,
                solana_program::clock::Clock::get()?.unix_timestamp,
            );
            pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;
            msg!("✅ PMUserAccount auto-initialized for {}", user_account.wallet);
        }

        // 7. 增加 PMUserAccount.prediction_market_locked (只增加 net_amount)
        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        pm_user_account.prediction_market_lock(net_amount as i64, solana_program::clock::Clock::get()?.unix_timestamp);
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        // 8. 如果有 fee，执行 Token Transfer (Vault → PM Fee Vault)
        if fee_amount > 0 {
            // Derive VaultConfig PDA for signing
            let (vault_config_pda, vault_config_bump) = Pubkey::find_program_address(
                &[b"vault_config"],
                program_id,
            );
            
            if vault_config_info.key != &vault_config_pda {
                msg!("❌ Invalid VaultConfig PDA");
                return Err(VaultError::InvalidPda.into());
            }
            
            let vault_config_seeds: &[&[u8]] = &[b"vault_config", &[vault_config_bump]];
            
            msg!("Transferring fee {} from Vault to PM Fee Vault", fee_amount);
            // 使用 token_compat 支持 Token-2022
            token_compat::transfer(
                token_program_info,
                vault_token_account_info,
                pm_fee_vault_info,
                vault_config_info,
                fee_amount,
                Some(vault_config_seeds),
            )?;
            
            // 9. 更新 PM Fee Config 统计 (累加 total_minting_fee)
            let mut pm_fee_config_data = pm_fee_config_info.try_borrow_mut_data()?;
            let current_total = i64::from_le_bytes(
                pm_fee_config_data[pm_fee_config_offsets::TOTAL_MINTING_FEE..pm_fee_config_offsets::TOTAL_MINTING_FEE + 8]
                    .try_into()
                    .unwrap()
            );
            let new_total = current_total.saturating_add(fee_amount as i64);
            pm_fee_config_data[pm_fee_config_offsets::TOTAL_MINTING_FEE..pm_fee_config_offsets::TOTAL_MINTING_FEE + 8]
                .copy_from_slice(&new_total.to_le_bytes());
            drop(pm_fee_config_data);
            
            msg!("✅ Minting fee {} collected (total: {})", fee_amount, new_total);
        }

        msg!("✅ PredictionMarketLockWithFee completed: gross={}, fee={}, net={}", 
             gross_amount, fee_amount, net_amount);
        Ok(())
    }

    /// 预测市场释放锁定并扣除手续费 (CPI only)
    /// 
    /// V2 Fee Architecture: 在 Vault 层面收取赎回手续费
    /// 
    /// Accounts:
    /// 0. `[]` VaultConfig
    /// 1. `[writable]` UserAccount
    /// 2. `[writable]` PredictionMarketUserAccount
    /// 3. `[]` Caller Program
    /// 4. `[writable]` Vault Token Account
    /// 5. `[writable]` PM Fee Vault
    /// 6. `[writable]` PM Fee Config PDA
    /// 7. `[]` Token Program
    fn process_prediction_market_unlock_with_fee(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        gross_amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;
        let vault_token_account_info = next_account_info(account_info_iter)?;
        let pm_fee_vault_info = next_account_info(account_info_iter)?;
        let pm_fee_config_info = next_account_info(account_info_iter)?;
        let token_program_info = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;
        assert_writable(pm_user_account_info)?;
        assert_writable(vault_token_account_info)?;
        assert_writable(pm_fee_vault_info)?;
        assert_writable(pm_fee_config_info)?;

        if gross_amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // 1. 验证 VaultConfig 和 CPI 调用方
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 2. 验证 Vault Token Account
        if vault_token_account_info.key != &vault_config.vault_token_account {
            msg!("❌ Invalid vault_token_account");
            return Err(VaultError::InvalidAccount.into());
        }

        // 3. 读取 PM Fee Config 获取费率
        let pm_fee_config_data = pm_fee_config_info.try_borrow_data()?;
        if pm_fee_config_data.len() < pm_fee_config_offsets::MIN_SIZE {
            msg!("❌ PM Fee Config not initialized");
            return Err(VaultError::InvalidAccount.into());
        }
        
        // 读取 redemption fee bps (offset 43, 2 bytes)
        let redemption_fee_bps = u16::from_le_bytes([
            pm_fee_config_data[pm_fee_config_offsets::REDEMPTION_FEE_BPS],
            pm_fee_config_data[pm_fee_config_offsets::REDEMPTION_FEE_BPS + 1],
        ]);
        
        // 读取 PM Fee Vault 地址用于验证
        let expected_fee_vault = Pubkey::new_from_array(
            pm_fee_config_data[pm_fee_config_offsets::FEE_VAULT..pm_fee_config_offsets::FEE_VAULT + 32]
                .try_into()
                .unwrap()
        );
        
        if pm_fee_vault_info.key != &expected_fee_vault {
            msg!("❌ PM Fee Vault mismatch");
            return Err(VaultError::InvalidAccount.into());
        }
        
        drop(pm_fee_config_data);

        // 4. 计算 fee 和 net_amount
        let fee_amount = ((gross_amount as u128) * (redemption_fee_bps as u128) / 10000) as u64;
        let net_amount = gross_amount.saturating_sub(fee_amount);
        
        msg!("PM Unlock with Fee: gross={}, fee_bps={}, fee={}, net={}", 
             gross_amount, redemption_fee_bps, fee_amount, net_amount);

        // 5. 从 PMUserAccount 扣除 gross_amount
        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        pm_user_account.prediction_market_unlock(gross_amount as i64, solana_program::clock::Clock::get()?.unix_timestamp)
            .map_err(|_| VaultError::InsufficientMargin)?;
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        // 6. 增加 UserAccount.available_balance (只增加 net_amount)
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, net_amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        // 7. 如果有 fee，执行 Token Transfer (Vault → PM Fee Vault)
        if fee_amount > 0 {
            let (vault_config_pda, vault_config_bump) = Pubkey::find_program_address(
                &[b"vault_config"],
                program_id,
            );
            
            if vault_config_info.key != &vault_config_pda {
                msg!("❌ Invalid VaultConfig PDA");
                return Err(VaultError::InvalidPda.into());
            }
            
            let vault_config_seeds: &[&[u8]] = &[b"vault_config", &[vault_config_bump]];
            
            msg!("Transferring fee {} from Vault to PM Fee Vault", fee_amount);
            // 使用 token_compat 支持 Token-2022
            token_compat::transfer(
                token_program_info,
                vault_token_account_info,
                pm_fee_vault_info,
                vault_config_info,
                fee_amount,
                Some(vault_config_seeds),
            )?;
            
            // 8. 更新 PM Fee Config 统计 (累加 total_redemption_fee)
            let mut pm_fee_config_data = pm_fee_config_info.try_borrow_mut_data()?;
            let current_total = i64::from_le_bytes(
                pm_fee_config_data[pm_fee_config_offsets::TOTAL_REDEMPTION_FEE..pm_fee_config_offsets::TOTAL_REDEMPTION_FEE + 8]
                    .try_into()
                    .unwrap()
            );
            let new_total = current_total.saturating_add(fee_amount as i64);
            pm_fee_config_data[pm_fee_config_offsets::TOTAL_REDEMPTION_FEE..pm_fee_config_offsets::TOTAL_REDEMPTION_FEE + 8]
                .copy_from_slice(&new_total.to_le_bytes());
            drop(pm_fee_config_data);
            
            msg!("✅ Redemption fee {} collected (total: {})", fee_amount, new_total);
        }

        msg!("✅ PredictionMarketUnlockWithFee completed: gross={}, fee={}, net={}", 
             gross_amount, fee_amount, net_amount);
        Ok(())
    }

    /// 预测市场交易费收取 (CPI only)
    /// 
    /// 仅收取交易费，不修改用户余额。余额调整由 PM Program 完成。
    fn process_prediction_market_trade_with_fee(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        trade_amount: u64,
        is_taker: bool,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        
        let vault_config_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;
        let vault_token_account_info = next_account_info(account_info_iter)?;
        let pm_fee_vault_info = next_account_info(account_info_iter)?;
        let pm_fee_config_info = next_account_info(account_info_iter)?;
        let token_program_info = next_account_info(account_info_iter)?;

        assert_writable(vault_token_account_info)?;
        assert_writable(pm_fee_vault_info)?;
        assert_writable(pm_fee_config_info)?;

        if trade_amount == 0 {
            msg!("Trade amount is 0, no fee to collect");
            return Ok(());
        }

        // 1. 验证 VaultConfig 和 CPI 调用方
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 2. 验证 Vault Token Account
        if vault_token_account_info.key != &vault_config.vault_token_account {
            msg!("❌ Invalid vault_token_account");
            return Err(VaultError::InvalidAccount.into());
        }

        // 3. 读取 PM Fee Config 获取费率
        // Taker fee at offset 45, Maker fee at offset 47
        const TAKER_FEE_BPS_OFFSET: usize = 45;
        const MAKER_FEE_BPS_OFFSET: usize = 47;
        const TOTAL_TRADING_FEE_OFFSET: usize = 73; // 57 + 8 + 8 = 73

        let pm_fee_config_data = pm_fee_config_info.try_borrow_data()?;
        if pm_fee_config_data.len() < pm_fee_config_offsets::MIN_SIZE {
            msg!("❌ PM Fee Config not initialized");
            return Err(VaultError::InvalidAccount.into());
        }
        
        let fee_bps = if is_taker {
            u16::from_le_bytes([
                pm_fee_config_data[TAKER_FEE_BPS_OFFSET],
                pm_fee_config_data[TAKER_FEE_BPS_OFFSET + 1],
            ])
        } else {
            u16::from_le_bytes([
                pm_fee_config_data[MAKER_FEE_BPS_OFFSET],
                pm_fee_config_data[MAKER_FEE_BPS_OFFSET + 1],
            ])
        };
        
        // 验证 PM Fee Vault
        let expected_fee_vault = Pubkey::new_from_array(
            pm_fee_config_data[pm_fee_config_offsets::FEE_VAULT..pm_fee_config_offsets::FEE_VAULT + 32]
                .try_into()
                .unwrap()
        );
        
        if pm_fee_vault_info.key != &expected_fee_vault {
            msg!("❌ PM Fee Vault mismatch");
            return Err(VaultError::InvalidAccount.into());
        }
        
        drop(pm_fee_config_data);

        // 4. 计算交易费
        let fee_amount = ((trade_amount as u128) * (fee_bps as u128) / 10000) as u64;
        
        msg!("PM Trade Fee: amount={}, is_taker={}, fee_bps={}, fee={}", 
             trade_amount, is_taker, fee_bps, fee_amount);

        // 5. 如果有 fee，执行 Token Transfer
        if fee_amount > 0 {
            let (vault_config_pda, vault_config_bump) = Pubkey::find_program_address(
                &[b"vault_config"],
                program_id,
            );
            
            if vault_config_info.key != &vault_config_pda {
                msg!("❌ Invalid VaultConfig PDA");
                return Err(VaultError::InvalidPda.into());
            }
            
            let vault_config_seeds: &[&[u8]] = &[b"vault_config", &[vault_config_bump]];
            
            msg!("Transferring trading fee {} from Vault to PM Fee Vault", fee_amount);
            // 使用 token_compat 支持 Token-2022
            token_compat::transfer(
                token_program_info,
                vault_token_account_info,
                pm_fee_vault_info,
                vault_config_info,
                fee_amount,
                Some(vault_config_seeds),
            )?;
            
            // 6. 更新 PM Fee Config 统计 (累加 total_trading_fee)
            let mut pm_fee_config_data = pm_fee_config_info.try_borrow_mut_data()?;
            let current_total = i64::from_le_bytes(
                pm_fee_config_data[TOTAL_TRADING_FEE_OFFSET..TOTAL_TRADING_FEE_OFFSET + 8]
                    .try_into()
                    .unwrap()
            );
            let new_total = current_total.saturating_add(fee_amount as i64);
            pm_fee_config_data[TOTAL_TRADING_FEE_OFFSET..TOTAL_TRADING_FEE_OFFSET + 8]
                .copy_from_slice(&new_total.to_le_bytes());
            drop(pm_fee_config_data);
            
            msg!("✅ Trading fee {} collected (total: {})", fee_amount, new_total);
        }

        msg!("✅ PredictionMarketTradeWithFee completed: amount={}, is_taker={}, fee={}", 
             trade_amount, is_taker, fee_amount);
        Ok(())
    }

    /// 预测市场结算并扣除手续费 (CPI only)
    fn process_prediction_market_settle_with_fee(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        locked_amount: u64,
        settlement_amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        
        let vault_config_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;
        let vault_token_account_info = next_account_info(account_info_iter)?;
        let pm_fee_vault_info = next_account_info(account_info_iter)?;
        let pm_fee_config_info = next_account_info(account_info_iter)?;
        let token_program_info = next_account_info(account_info_iter)?;

        assert_writable(pm_user_account_info)?;
        assert_writable(vault_token_account_info)?;
        assert_writable(pm_fee_vault_info)?;
        assert_writable(pm_fee_config_info)?;

        // 1. 验证 VaultConfig 和 CPI 调用方
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 2. 验证 Vault Token Account
        if vault_token_account_info.key != &vault_config.vault_token_account {
            msg!("❌ Invalid vault_token_account");
            return Err(VaultError::InvalidAccount.into());
        }

        // 3. 读取 PM Fee Config 获取结算费率
        const SETTLEMENT_FEE_BPS_OFFSET: usize = 49;
        
        let pm_fee_config_data = pm_fee_config_info.try_borrow_data()?;
        if pm_fee_config_data.len() < pm_fee_config_offsets::MIN_SIZE {
            msg!("❌ PM Fee Config not initialized");
            return Err(VaultError::InvalidAccount.into());
        }
        
        let settlement_fee_bps = u16::from_le_bytes([
            pm_fee_config_data[SETTLEMENT_FEE_BPS_OFFSET],
            pm_fee_config_data[SETTLEMENT_FEE_BPS_OFFSET + 1],
        ]);
        
        // 验证 PM Fee Vault
        let expected_fee_vault = Pubkey::new_from_array(
            pm_fee_config_data[pm_fee_config_offsets::FEE_VAULT..pm_fee_config_offsets::FEE_VAULT + 32]
                .try_into()
                .unwrap()
        );
        
        if pm_fee_vault_info.key != &expected_fee_vault {
            msg!("❌ PM Fee Vault mismatch");
            return Err(VaultError::InvalidAccount.into());
        }
        
        drop(pm_fee_config_data);

        // 4. 计算 fee 和 net_settlement
        let fee_amount = ((settlement_amount as u128) * (settlement_fee_bps as u128) / 10000) as u64;
        let net_settlement = settlement_amount.saturating_sub(fee_amount);
        
        msg!("PM Settle with Fee: locked={}, settlement={}, fee_bps={}, fee={}, net={}", 
             locked_amount, settlement_amount, settlement_fee_bps, fee_amount, net_settlement);

        // 5. 从 PMUserAccount 扣除 locked_amount，记入 net_settlement
        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        
        // 扣除 locked
        pm_user_account.prediction_market_locked_e6 = checked_sub(
            pm_user_account.prediction_market_locked_e6,
            locked_amount as i64
        )?;
        
        // 增加 pending_settlement (净额)
        pm_user_account.prediction_market_pending_settlement_e6 = checked_add(
            pm_user_account.prediction_market_pending_settlement_e6,
            net_settlement as i64
        )?;
        
        pm_user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        // 6. 如果有 fee，执行 Token Transfer
        if fee_amount > 0 {
            let (vault_config_pda, vault_config_bump) = Pubkey::find_program_address(
                &[b"vault_config"],
                program_id,
            );
            
            if vault_config_info.key != &vault_config_pda {
                msg!("❌ Invalid VaultConfig PDA");
                return Err(VaultError::InvalidPda.into());
            }
            
            let vault_config_seeds: &[&[u8]] = &[b"vault_config", &[vault_config_bump]];
            
            // 注意: 结算费从 Vault 转出，因为用户的收益本质上是其他用户的损失
            // 在 Complete Set 机制中，总资金是守恒的
            msg!("Transferring settlement fee {} from Vault to PM Fee Vault", fee_amount);
            // 使用 token_compat 支持 Token-2022
            token_compat::transfer(
                token_program_info,
                vault_token_account_info,
                pm_fee_vault_info,
                vault_config_info,
                fee_amount,
                Some(vault_config_seeds),
            )?;
            
            msg!("✅ Settlement fee {} collected", fee_amount);
        }

        msg!("✅ PredictionMarketSettleWithFee completed: locked={}, settlement={}, fee={}, net={}", 
             locked_amount, settlement_amount, fee_amount, net_settlement);
        Ok(())
    }

    // =========================================================================
    // Spot 交易指令处理
    // =========================================================================

    /// 初始化 Spot 用户账户
    fn process_initialize_spot_user(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        // 验证 PDA
        let (spot_user_pda, spot_user_bump) = Pubkey::find_program_address(
            &[SPOT_USER_SEED, user.key.as_ref()],
            program_id,
        );

        if spot_user_account_info.key != &spot_user_pda {
            msg!("❌ Invalid SpotUserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        // 检查是否已初始化
        if !spot_user_account_info.data_is_empty() {
            msg!("SpotUserAccount already initialized");
            return Err(VaultError::AlreadyInitialized.into());
        }

        // 创建账户
        let rent = Rent::get()?;
        let space = SPOT_USER_ACCOUNT_SIZE;
        let lamports = rent.minimum_balance(space);

        invoke_signed(
            &system_instruction::create_account(
                user.key,
                spot_user_account_info.key,
                lamports,
                space as u64,
                program_id,
            ),
            &[user.clone(), spot_user_account_info.clone(), system_program.clone()],
            &[&[SPOT_USER_SEED, user.key.as_ref(), &[spot_user_bump]]],
        )?;

        // 初始化数据
        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        let spot_user = SpotUserAccount::new(*user.key, spot_user_bump, current_ts);
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotUserAccount initialized for {}", user.key);
        Ok(())
    }

    /// Spot Token 入金 (用户直接调用)
    fn process_spot_deposit(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let _vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        // 验证 SpotUserAccount 所有权
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        if spot_user.wallet != *user.key {
            return Err(VaultError::UnauthorizedUser.into());
        }

        // 执行 Token 转账 - 使用 token_compat 支持 Token-2022
        token_compat::transfer(
            token_program,
            user_token_account,
            vault_token_account,
            user,
            amount,
            None, // 用户签名，不需要 PDA seeds
        )?;

        // 更新 SpotUserAccount 余额
        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        spot_user.deposit(token_index, amount as i64, current_ts)
            .map_err(|_| VaultError::DepositFailed)?;
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotDeposit: token_index={}, amount={}", token_index, amount);
        Ok(())
    }

    /// Spot Token 出金 (用户直接调用)
    fn process_spot_withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        // 验证 SpotUserAccount 所有权
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        if spot_user.wallet != *user.key {
            return Err(VaultError::UnauthorizedUser.into());
        }

        // 检查余额
        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        spot_user.withdraw(token_index, amount as i64, current_ts)
            .map_err(|_| VaultError::InsufficientBalance)?;

        // 获取 VaultConfig PDA 用于签名
        let (vault_config_pda, vault_config_bump) = Pubkey::find_program_address(
            &[b"vault_config"],
            program_id,
        );
        if vault_config_info.key != &vault_config_pda {
            return Err(VaultError::InvalidPda.into());
        }

        // 执行 Token 转账 - 使用 token_compat 支持 Token-2022
        token_compat::transfer(
            token_program,
            vault_token_account,
            user_token_account,
            vault_config_info,
            amount,
            Some(&[b"vault_config", &[vault_config_bump]]),
        )?;

        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotWithdraw: token_index={}, amount={}", token_index, amount);
        Ok(())
    }

    /// Spot 锁定余额 (CPI only)
    fn process_spot_lock_balance(
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        // 验证 CPI 调用方
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 锁定余额
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        spot_user.lock_balance(token_index, amount as i64, current_ts)
            .map_err(|_| VaultError::InsufficientBalance)?;
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotLockBalance: token_index={}, amount={}", token_index, amount);
        Ok(())
    }

    /// Spot 解锁余额 (CPI only)
    fn process_spot_unlock_balance(
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        // 验证 CPI 调用方
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 解锁余额
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        spot_user.unlock_balance(token_index, amount as i64, current_ts)
            .map_err(|_| VaultError::InsufficientBalance)?;
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotUnlockBalance: token_index={}, amount={}", token_index, amount);
        Ok(())
    }

    /// Spot 交易结算 (CPI only)
    fn process_spot_settle_trade(
        accounts: &[AccountInfo],
        is_buy: bool,
        base_token_index: u16,
        quote_token_index: u16,
        base_amount: u64,
        quote_amount: u64,
        sequence: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        // 验证 CPI 调用方
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // 执行结算
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        spot_user.settle_trade(
            is_buy,
            base_token_index,
            quote_token_index,
            base_amount as i64,
            quote_amount as i64,
            sequence,
            current_ts,
        ).map_err(|e| {
            msg!("SpotSettleTrade error: {}", e);
            VaultError::SettlementFailed
        })?;
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotSettleTrade: is_buy={}, base={}, quote={}, seq={}", 
             is_buy, base_amount, quote_amount, sequence);
        Ok(())
    }

    /// Relayer 代理 Spot 入金
    fn process_relayer_spot_deposit(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(admin)?;

        // 验证 Admin
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证/创建 SpotUserAccount PDA
        let (spot_user_pda, spot_user_bump) = Pubkey::find_program_address(
            &[SPOT_USER_SEED, user_wallet.as_ref()],
            program_id,
        );

        if spot_user_account_info.key != &spot_user_pda {
            return Err(VaultError::InvalidPda.into());
        }

        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;

        // 如果账户不存在则创建
        if spot_user_account_info.data_is_empty() {
            let rent = Rent::get()?;
            let space = SPOT_USER_ACCOUNT_SIZE;
            let lamports = rent.minimum_balance(space);

            invoke_signed(
                &system_instruction::create_account(
                    admin.key,
                    spot_user_account_info.key,
                    lamports,
                    space as u64,
                    program_id,
                ),
                &[admin.clone(), spot_user_account_info.clone(), system_program.clone()],
                &[&[SPOT_USER_SEED, user_wallet.as_ref(), &[spot_user_bump]]],
            )?;

            let spot_user = SpotUserAccount::new(user_wallet, spot_user_bump, current_ts);
            spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;
            msg!("Created SpotUserAccount for {}", user_wallet);
        }

        // 增加余额
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        spot_user.deposit(token_index, amount as i64, current_ts)
            .map_err(|_| VaultError::DepositFailed)?;
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ RelayerSpotDeposit: user={}, token_index={}, amount={}", user_wallet, token_index, amount);
        Ok(())
    }

    /// Relayer 代理 Spot 出金
    fn process_relayer_spot_withdraw(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;

        // 验证 Admin
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证 SpotUserAccount 所有权
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        if spot_user.wallet != user_wallet {
            return Err(VaultError::UnauthorizedUser.into());
        }

        // 扣除余额
        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        spot_user.withdraw(token_index, amount as i64, current_ts)
            .map_err(|_| VaultError::InsufficientBalance)?;
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ RelayerSpotWithdraw: user={}, token_index={}, amount={}", user_wallet, token_index, amount);
        Ok(())
    }

    // =========================================================================
    // Spot 统一账户指令处理函数 (2025-12-31 新增)
    // =========================================================================

    /// Relayer 代理 Spot 交易结算
    /// 
    /// CEX 级体验：同时更新 Maker 和 Taker 两个 SpotUserAccount
    /// 优先从 available_e6 扣除，符合 Hyperliquid 模式
    fn process_relayer_spot_settle_trade(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        maker_wallet: Pubkey,
        taker_wallet: Pubkey,
        base_token_index: u16,
        quote_token_index: u16,
        base_amount_e6: i64,
        quote_amount_e6: i64,
        maker_fee_e6: i64,
        taker_fee_e6: i64,
        taker_is_buy: bool,
        sequence: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let maker_spot_account_info = next_account_info(account_info_iter)?;
        let taker_spot_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;

        // 验证 Admin
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证 Maker SpotUserAccount PDA
        let (maker_pda, _) = Pubkey::find_program_address(
            &[SPOT_USER_SEED, maker_wallet.as_ref()],
            program_id,
        );
        if maker_spot_account_info.key != &maker_pda {
            msg!("❌ Invalid Maker SpotUserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        // 验证 Taker SpotUserAccount PDA
        let (taker_pda, _) = Pubkey::find_program_address(
            &[SPOT_USER_SEED, taker_wallet.as_ref()],
            program_id,
        );
        if taker_spot_account_info.key != &taker_pda {
            msg!("❌ Invalid Taker SpotUserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;

        // 更新 Maker 余额
        let mut maker_spot = deserialize_account::<SpotUserAccount>(&maker_spot_account_info.data.borrow())?;
        maker_spot.settle_trade_v2(
            !taker_is_buy,  // Maker 方向与 Taker 相反
            base_token_index,
            quote_token_index,
            base_amount_e6,
            quote_amount_e6,
            maker_fee_e6,
            sequence,
            current_ts,
        ).map_err(|e| {
            msg!("❌ Maker settle_trade_v2 failed: {}", e);
            VaultError::SettlementFailed
        })?;
        maker_spot.serialize(&mut &mut maker_spot_account_info.data.borrow_mut()[..])?;

        // 更新 Taker 余额
        let mut taker_spot = deserialize_account::<SpotUserAccount>(&taker_spot_account_info.data.borrow())?;
        taker_spot.settle_trade_v2(
            taker_is_buy,
            base_token_index,
            quote_token_index,
            base_amount_e6,
            quote_amount_e6,
            taker_fee_e6,
            sequence,
            current_ts,
        ).map_err(|e| {
            msg!("❌ Taker settle_trade_v2 failed: {}", e);
            VaultError::SettlementFailed
        })?;
        taker_spot.serialize(&mut &mut taker_spot_account_info.data.borrow_mut()[..])?;

        msg!("✅ RelayerSpotSettleTrade: maker={}, taker={}, base={}, quote={}, seq={}",
             maker_wallet, taker_wallet, base_amount_e6, quote_amount_e6, sequence);
        Ok(())
    }

    /// 从 UserAccount 划转 USDC 到 SpotUserAccount
    fn process_spot_allocate_from_vault(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(admin)?;

        // 验证 Admin
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证 UserAccount PDA (seed: ["user", wallet])
        let (user_pda, _) = Pubkey::find_program_address(
            &[b"user", user_wallet.as_ref()],
            program_id,
        );
        if user_account_info.key != &user_pda {
            msg!("❌ Invalid UserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        // 验证 SpotUserAccount PDA
        let (spot_user_pda, spot_user_bump) = Pubkey::find_program_address(
            &[SPOT_USER_SEED, user_wallet.as_ref()],
            program_id,
        );
        if spot_user_account_info.key != &spot_user_pda {
            msg!("❌ Invalid SpotUserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;

        // 从 UserAccount 扣除 USDC
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        if user_account.available_balance_e6 < amount as i64 {
            msg!("❌ Insufficient UserAccount balance: available={}, required={}",
                 user_account.available_balance_e6, amount);
            return Err(VaultError::InsufficientBalance.into());
        }
        user_account.available_balance_e6 -= amount as i64;
        user_account.last_update_ts = current_ts;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        // 如果 SpotUserAccount 不存在则创建
        if spot_user_account_info.data_is_empty() {
            let rent = Rent::get()?;
            let space = SPOT_USER_ACCOUNT_SIZE;
            let lamports = rent.minimum_balance(space);

            invoke_signed(
                &system_instruction::create_account(
                    admin.key,
                    spot_user_account_info.key,
                    lamports,
                    space as u64,
                    program_id,
                ),
                &[admin.clone(), spot_user_account_info.clone(), system_program.clone()],
                &[&[SPOT_USER_SEED, user_wallet.as_ref(), &[spot_user_bump]]],
            )?;

            let spot_user = SpotUserAccount::new(user_wallet, spot_user_bump, current_ts);
            spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;
            msg!("Created SpotUserAccount for {}", user_wallet);
        }

        // 增加 SpotUserAccount USDC 余额 (token_index=0)
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        spot_user.deposit(0, amount as i64, current_ts)  // USDC = token_index 0
            .map_err(|_| VaultError::DepositFailed)?;
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotAllocateFromVault: user={}, amount={}", user_wallet, amount);
        Ok(())
    }

    /// 从 SpotUserAccount 划转 USDC 到 UserAccount
    fn process_spot_release_to_vault(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let spot_user_account_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;

        // 验证 Admin
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证 SpotUserAccount PDA
        let (spot_user_pda, _) = Pubkey::find_program_address(
            &[SPOT_USER_SEED, user_wallet.as_ref()],
            program_id,
        );
        if spot_user_account_info.key != &spot_user_pda {
            msg!("❌ Invalid SpotUserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        // 验证 UserAccount PDA
        let (user_pda, _) = Pubkey::find_program_address(
            &[b"user", user_wallet.as_ref()],
            program_id,
        );
        if user_account_info.key != &user_pda {
            msg!("❌ Invalid UserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;

        // 从 SpotUserAccount 扣除 USDC (token_index=0)
        let mut spot_user = deserialize_account::<SpotUserAccount>(&spot_user_account_info.data.borrow())?;
        spot_user.withdraw(0, amount as i64, current_ts)  // USDC = token_index 0
            .map_err(|e| {
                msg!("❌ SpotUserAccount withdraw failed: {}", e);
                VaultError::InsufficientBalance
            })?;
        spot_user.serialize(&mut &mut spot_user_account_info.data.borrow_mut()[..])?;

        // 增加 UserAccount USDC 余额
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        user_account.available_balance_e6 += amount as i64;
        user_account.last_update_ts = current_ts;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotReleaseToVault: user={}, amount={}", user_wallet, amount);
        Ok(())
    }

    // =========================================================================
    // 站内支付相关处理函数
    // =========================================================================

    /// 处理 Relayer 代理内部转账
    /// 
    /// 流程:
    /// 1. 验证 Admin/Relayer 签名
    /// 2. 加载发送方和接收方 UserAccount
    /// 3. 验证发送方余额 >= amount + fee
    /// 4. 扣减: from_account.available_balance -= (amount + fee)
    /// 5. 增加: to_account.available_balance += amount
    /// 6. 手续费进入 Insurance Fund (记账)
    fn process_relayer_internal_transfer(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        from_wallet: Pubkey,
        to_wallet: Pubkey,
        amount: u64,
        fee: u64,
        transfer_type: u8,
        reference_hash: [u8; 32],
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let from_account_info = next_account_info(account_info_iter)?;
        let to_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        // 验证 Admin 签名
        if !admin.is_signer {
            msg!("Admin must sign the transaction");
            return Err(VaultError::MissingSignature.into());
        }

        // 加载 VaultConfig
        let vault_config: VaultConfig = deserialize_account(&vault_config_info.data.borrow())?;

        // 验证 Admin 权限
        if vault_config.admin != *admin.key {
            msg!("Only admin can call RelayerInternalTransfer");
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证 from UserAccount PDA
        let (expected_from_pda, from_bump) = Pubkey::find_program_address(
            &[b"user", from_wallet.as_ref()],
            program_id,
        );
        if from_account_info.key != &expected_from_pda {
            msg!("Invalid from_account PDA");
            return Err(VaultError::InvalidUserAccount.into());
        }

        // 验证 to UserAccount PDA
        let (expected_to_pda, _to_bump) = Pubkey::find_program_address(
            &[b"user", to_wallet.as_ref()],
            program_id,
        );
        if to_account_info.key != &expected_to_pda {
            msg!("Invalid to_account PDA");
            return Err(VaultError::InvalidUserAccount.into());
        }

        // 加载并更新 from UserAccount
        let mut from_account: UserAccount = deserialize_account(&from_account_info.data.borrow())?;
        let total_deduction = (amount + fee) as i64;
        
        if from_account.available_balance_e6 < total_deduction {
            msg!("Insufficient balance: available={}, required={}", 
                from_account.available_balance_e6, total_deduction);
            return Err(VaultError::InsufficientBalance.into());
        }

        from_account.available_balance_e6 -= total_deduction;
        from_account.last_update_ts = get_current_timestamp();

        // 序列化 from UserAccount
        from_account.serialize(&mut &mut from_account_info.data.borrow_mut()[..])?;

        // 加载并更新 to UserAccount
        let mut to_account: UserAccount = deserialize_account(&to_account_info.data.borrow())?;
        to_account.available_balance_e6 += amount as i64;
        to_account.last_update_ts = get_current_timestamp();

        // 序列化 to UserAccount
        to_account.serialize(&mut &mut to_account_info.data.borrow_mut()[..])?;

        msg!("✅ RelayerInternalTransfer: from={}, to={}, amount={}, fee={}, type={}, ref={:?}",
            from_wallet, to_wallet, amount, fee, transfer_type, &reference_hash[..8]);
        Ok(())
    }

    /// 处理初始化定时支付授权
    fn process_init_recurring_auth(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        payer: Pubkey,
        payee: Pubkey,
        amount: u64,
        interval_seconds: i64,
        max_cycles: u32,
        registration_fee: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let payer_account_info = next_account_info(account_info_iter)?;
        let recurring_auth_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        // 验证 Admin 签名
        if !admin.is_signer {
            return Err(VaultError::MissingSignature.into());
        }

        // 加载 VaultConfig
        let vault_config: VaultConfig = deserialize_account(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证 payer UserAccount PDA
        let (expected_payer_pda, _) = Pubkey::find_program_address(
            &[b"user", payer.as_ref()],
            program_id,
        );
        if payer_account_info.key != &expected_payer_pda {
            return Err(VaultError::InvalidUserAccount.into());
        }

        // 验证 RecurringAuth PDA
        let (expected_recurring_pda, recurring_bump) = Pubkey::find_program_address(
            &[RECURRING_AUTH_SEED, payer.as_ref(), payee.as_ref()],
            program_id,
        );
        if recurring_auth_info.key != &expected_recurring_pda {
            return Err(VaultError::InvalidPda.into());
        }

        // 扣除注册手续费
        let mut payer_account: UserAccount = deserialize_account(&payer_account_info.data.borrow())?;
        if payer_account.available_balance_e6 < registration_fee as i64 {
            msg!("Insufficient balance for registration fee");
            return Err(VaultError::InsufficientBalance.into());
        }
        payer_account.available_balance_e6 -= registration_fee as i64;
        payer_account.last_update_ts = get_current_timestamp();
        payer_account.serialize(&mut &mut payer_account_info.data.borrow_mut()[..])?;

        // 创建 RecurringAuth PDA
        let rent = Rent::get()?;
        let space = RECURRING_AUTH_SIZE;
        let lamports = rent.minimum_balance(space);

        let seeds = &[
            RECURRING_AUTH_SEED,
            payer.as_ref(),
            payee.as_ref(),
            &[recurring_bump],
        ];

        invoke_signed(
            &system_instruction::create_account(
                admin.key,
                recurring_auth_info.key,
                lamports,
                space as u64,
                program_id,
            ),
            &[admin.clone(), recurring_auth_info.clone(), system_program.clone()],
            &[seeds],
        )?;

        // 初始化 RecurringAuth
        let recurring_auth = RecurringAuth::new(
            payer,
            payee,
            recurring_bump,
            amount,
            interval_seconds,
            max_cycles,
            get_current_timestamp(),
        );
        recurring_auth.serialize(&mut &mut recurring_auth_info.data.borrow_mut()[..])?;

        msg!("✅ InitRecurringAuth: payer={}, payee={}, amount={}, interval={}s, fee={}",
            payer, payee, amount, interval_seconds, registration_fee);
        Ok(())
    }

    /// 处理执行定时支付
    fn process_execute_recurring_payment(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        payer: Pubkey,
        payee: Pubkey,
        amount: u64,
        fee: u64,
        cycle_count: u32,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let payer_account_info = next_account_info(account_info_iter)?;
        let payee_account_info = next_account_info(account_info_iter)?;
        let recurring_auth_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        // 验证 Admin 签名
        if !admin.is_signer {
            return Err(VaultError::MissingSignature.into());
        }

        let vault_config: VaultConfig = deserialize_account(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证 PDAs
        let (expected_payer_pda, _) = Pubkey::find_program_address(
            &[b"user", payer.as_ref()],
            program_id,
        );
        if payer_account_info.key != &expected_payer_pda {
            return Err(VaultError::InvalidUserAccount.into());
        }

        let (expected_payee_pda, _) = Pubkey::find_program_address(
            &[b"user", payee.as_ref()],
            program_id,
        );
        if payee_account_info.key != &expected_payee_pda {
            return Err(VaultError::InvalidUserAccount.into());
        }

        let (expected_recurring_pda, _) = Pubkey::find_program_address(
            &[RECURRING_AUTH_SEED, payer.as_ref(), payee.as_ref()],
            program_id,
        );
        if recurring_auth_info.key != &expected_recurring_pda {
            return Err(VaultError::InvalidPda.into());
        }

        // 加载 RecurringAuth
        let mut recurring_auth: RecurringAuth = deserialize_account(&recurring_auth_info.data.borrow())?;
        if !recurring_auth.is_active {
            msg!("RecurringAuth is not active");
            return Err(VaultError::RecurringAuthNotActive.into());
        }

        // 验证 cycle_count
        if cycle_count != recurring_auth.current_cycles + 1 {
            msg!("Invalid cycle count: expected {}, got {}", 
                recurring_auth.current_cycles + 1, cycle_count);
            return Err(VaultError::InvalidCycleCount.into());
        }

        // 扣除 payer 余额
        let mut payer_account: UserAccount = deserialize_account(&payer_account_info.data.borrow())?;
        let total_deduction = (amount + fee) as i64;
        
        if payer_account.available_balance_e6 < total_deduction {
            return Err(VaultError::InsufficientBalance.into());
        }
        payer_account.available_balance_e6 -= total_deduction;
        payer_account.last_update_ts = get_current_timestamp();
        payer_account.serialize(&mut &mut payer_account_info.data.borrow_mut()[..])?;

        // 增加 payee 余额
        let mut payee_account: UserAccount = deserialize_account(&payee_account_info.data.borrow())?;
        payee_account.available_balance_e6 += amount as i64;
        payee_account.last_update_ts = get_current_timestamp();
        payee_account.serialize(&mut &mut payee_account_info.data.borrow_mut()[..])?;

        // 更新 RecurringAuth
        recurring_auth.execute(get_current_timestamp())
            .map_err(|_| VaultError::RecurringAuthExecutionFailed)?;
        recurring_auth.serialize(&mut &mut recurring_auth_info.data.borrow_mut()[..])?;

        msg!("✅ ExecuteRecurringPayment: payer={}, payee={}, amount={}, fee={}, cycle={}",
            payer, payee, amount, fee, cycle_count);
        Ok(())
    }

    /// 处理取消定时支付授权
    fn process_cancel_recurring_auth(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        payer: Pubkey,
        payee: Pubkey,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let recurring_auth_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        // 验证 Admin 签名
        if !admin.is_signer {
            return Err(VaultError::MissingSignature.into());
        }

        let vault_config: VaultConfig = deserialize_account(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        // 验证 RecurringAuth PDA
        let (expected_recurring_pda, _) = Pubkey::find_program_address(
            &[RECURRING_AUTH_SEED, payer.as_ref(), payee.as_ref()],
            program_id,
        );
        if recurring_auth_info.key != &expected_recurring_pda {
            return Err(VaultError::InvalidPda.into());
        }

        // 取消授权
        let mut recurring_auth: RecurringAuth = deserialize_account(&recurring_auth_info.data.borrow())?;
        recurring_auth.cancel();
        recurring_auth.serialize(&mut &mut recurring_auth_info.data.borrow_mut()[..])?;

        msg!("✅ CancelRecurringAuth: payer={}, payee={}", payer, payee);
        Ok(())
    }
}

/// Program entrypoint's implementation
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    Processor::process(program_id, accounts, instruction_data)
}
