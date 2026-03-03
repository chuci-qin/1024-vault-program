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
    program_error::ProgramError,
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

/// 验证调用方：支持 Admin 签名（直接调用）或 CPI 调用方（跨程序调用）
/// 
/// Spot 系列指令需要同时支持两种调用模式：
/// 1. Admin/Relayer 直接签名调用（后端 SpotBackend 发送交易）
/// 2. 其他链上程序通过 CPI 调用（未来扩展）
fn verify_admin_or_cpi_caller(
    vault_config: &VaultConfig,
    caller: &AccountInfo,
) -> ProgramResult {
    if caller.is_signer && caller.key == &vault_config.admin {
        msg!("✅ Caller verified as admin signer");
        return Ok(());
    }
    
    if vault_config.is_authorized_caller(caller.key) {
        verify_cpi_caller(vault_config, caller)?;
        return Ok(());
    }
    
    msg!("❌ Caller {} is neither admin signer nor authorized CPI caller", caller.key);
    Err(VaultError::UnauthorizedCaller.into())
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
            VaultInstruction::Deprecated_InitializeSpotUser => {
                msg!("Instruction: Deprecated_InitializeSpotUser");
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
                Self::process_spot_lock_balance(program_id, accounts, token_index, amount)
            }
            VaultInstruction::SpotUnlockBalance { token_index, amount } => {
                msg!("Instruction: SpotUnlockBalance");
                Self::process_spot_unlock_balance(program_id, accounts, token_index, amount)
            }
            VaultInstruction::Deprecated_SpotSettleTrade { _is_buy, _base_token_index, _quote_token_index, _base_amount, _quote_amount, _sequence } => {
                msg!("Instruction: Deprecated_SpotSettleTrade");
                Self::process_spot_settle_trade(accounts, _is_buy, _base_token_index, _quote_token_index, _base_amount, _quote_amount, _sequence)
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
            VaultInstruction::CreditUserBalance { user_wallet, amount } => {
                msg!("Instruction: CreditUserBalance");
                Self::process_credit_user_balance(program_id, accounts, user_wallet, amount)
            }
            VaultInstruction::PredictionMarketSettleToAvailable { locked_amount, settlement_amount } => {
                msg!("Instruction: PredictionMarketSettleToAvailable");
                Self::process_prediction_market_settle_to_available(program_id, accounts, locked_amount, settlement_amount)
            }
            VaultInstruction::RelayerPredictionMarketClaimSettlement => {
                msg!("Instruction: RelayerPredictionMarketClaimSettlement");
                Self::process_relayer_prediction_market_claim_settlement(accounts)
            }
            VaultInstruction::RelayerWithdrawAndTransfer { user_wallet, amount } => {
                msg!("Instruction: RelayerWithdrawAndTransfer");
                Self::process_relayer_withdraw_and_transfer(program_id, accounts, user_wallet, amount)
            }
            // One Account Experience — Spot USDC 统一管理
            VaultInstruction::SpotLockUsdc { amount } => {
                msg!("Instruction: SpotLockUsdc");
                Self::process_spot_lock_usdc(accounts, amount)
            }
            VaultInstruction::SpotUnlockUsdc { amount } => {
                msg!("Instruction: SpotUnlockUsdc");
                Self::process_spot_unlock_usdc(accounts, amount)
            }
            VaultInstruction::SpotSettleUsdcTrade { buyer_usdc, seller_credit, buyer_fee, seller_fee, base_amount, sequence, base_token_index } => {
                msg!("Instruction: SpotSettleUsdcTrade");
                Self::process_spot_settle_usdc_trade(program_id, accounts, buyer_usdc, seller_credit, buyer_fee, seller_fee, base_amount, sequence, base_token_index)
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
            spot_locked_e6: 0,
            reserved: [0; 56],
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
            
            // G5 A2: 删除真实 USDC 转账（纯记账模式 — 清算罚金仅通过 InsuranceFundConfig 统计追踪）
            // 原: token_compat::transfer(... vault → insurance_fund_vault ...)
            msg!("Liquidation penalty {} recorded (pure accounting, no transfer)", liquidation_penalty);
            let _ = insurance_fund_vault; // suppress unused
            let _ = token_program;
            let _ = bump;
            
            msg!(
                "✅ Liquidation penalty {} recorded to Insurance Fund (accounting only)",
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

    /// 预测市场结算直接到可用余额 (CPI only)
    /// pm_locked -= locked_amount, available += settlement_amount
    fn process_prediction_market_settle_to_available(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        locked_amount: u64,
        settlement_amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;
        assert_writable(pm_user_account_info)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        // Release locked amount from PMUserAccount
        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        if locked_amount > 0 {
            if pm_user_account.prediction_market_locked_e6 < locked_amount as i64 {
                msg!("SettleToAvailable: insufficient pm_locked {} < {}", 
                     pm_user_account.prediction_market_locked_e6, locked_amount);
                return Err(VaultError::InsufficientMargin.into());
            }
            pm_user_account.prediction_market_locked_e6 -= locked_amount as i64;
        }
        pm_user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        // Credit settlement to UserAccount.available_balance
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        if user_account.wallet != pm_user_account.wallet {
            msg!("SettleToAvailable: wallet mismatch user={} pm={}", user_account.wallet, pm_user_account.wallet);
            return Err(VaultError::InvalidAccount.into());
        }
        if settlement_amount > 0 {
            user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, settlement_amount as i64)?;
        }
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("PM SettleToAvailable: locked={}, settlement={}", locked_amount, settlement_amount);
        Ok(())
    }

    /// Relayer 代替用户领取历史 pending_settlement
    fn process_relayer_prediction_market_claim_settlement(
        accounts: &[AccountInfo],
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller = next_account_info(account_info_iter)?;

        assert_signer(caller)?;
        assert_writable(user_account_info)?;
        assert_writable(pm_user_account_info)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *caller.key {
            return Err(VaultError::InvalidAdmin.into());
        }

        let mut pm_user_account = deserialize_account::<PredictionMarketUserAccount>(&pm_user_account_info.data.borrow())?;
        let claim_amount = pm_user_account.prediction_market_claim_settlement(
            solana_program::clock::Clock::get()?.unix_timestamp
        );
        pm_user_account.serialize(&mut &mut pm_user_account_info.data.borrow_mut()[..])?;

        if claim_amount <= 0 {
            msg!("No pending settlement to claim");
            return Ok(());
        }

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        if user_account.wallet != pm_user_account.wallet {
            return Err(VaultError::InvalidAccount.into());
        }
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, claim_amount)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("Relayer claimed PM settlement: {} e6", claim_amount);
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
                spot_locked_e6: 0,
                reserved: [0; 56],
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

    /// Relayer 代理出金并转账
    ///
    /// 功能：
    /// 1. 验证 Admin 签名
    /// 2. 扣除用户 Vault 余额
    /// 3. 从 Vault Token Account 转 USDC 到 Relayer Token Account
    ///
    /// Accounts:
    /// 0. `[signer]` Admin/Relayer
    /// 1. `[writable]` UserAccount PDA
    /// 2. `[]` VaultConfig
    /// 3. `[writable]` Vault Token Account
    /// 4. `[writable]` Relayer Token Account
    /// 5. `[]` Token Program
    fn process_relayer_withdraw_and_transfer(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let relayer_token_account = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        assert_writable(user_account_info)?;
        assert_writable(vault_token_account)?;
        assert_writable(relayer_token_account)?;

        let vault_config_data = vault_config_info.data.borrow();
        if vault_config_data.len() < 40 {
            msg!("❌ Invalid VaultConfig data length: {}", vault_config_data.len());
            return Err(VaultError::InvalidAccount.into());
        }

        let stored_admin = Pubkey::try_from(&vault_config_data[8..40])
            .map_err(|_| VaultError::InvalidAccount)?;

        if stored_admin != *admin.key {
            msg!("❌ Invalid relayer: {} (expected admin: {})", admin.key, stored_admin);
            return Err(VaultError::InvalidRelayer.into());
        }

        drop(vault_config_data);

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let (user_account_pda, _bump) = Pubkey::find_program_address(
            &[b"user", user_wallet.as_ref()],
            program_id
        );
        if user_account_info.key != &user_account_pda {
            msg!("❌ Invalid UserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        if user_account_info.data_is_empty() {
            msg!("❌ UserAccount does not exist for {}", user_wallet);
            return Err(VaultError::NotInitialized.into());
        }

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;

        if user_account.wallet != user_wallet {
            msg!("❌ Wallet mismatch: expected {}, got {}", user_wallet, user_account.wallet);
            return Err(VaultError::InvalidAccount.into());
        }

        if user_account.available_balance_e6 < amount as i64 {
            msg!("❌ Insufficient balance: {} < {}", user_account.available_balance_e6, amount);
            return Err(VaultError::InsufficientBalance.into());
        }

        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, amount as i64)?;
        user_account.total_withdrawn_e6 = checked_add(user_account.total_withdrawn_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        let (_vault_config_pda, vault_config_bump) =
            Pubkey::find_program_address(&[b"vault_config"], program_id);

        token_compat::transfer(
            token_program,
            vault_token_account,
            relayer_token_account,
            vault_config_info,
            amount,
            Some(&[b"vault_config", &[vault_config_bump]]),
        )?;

        msg!("✅ RelayerWithdrawAndTransfer {} e6 for {} → relayer {} (remaining: {})",
            amount, user_wallet, admin.key, user_account.available_balance_e6);

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
            
            let _vault_config_seeds: &[&[u8]] = &[b"vault_config", &[vault_config_bump]];
            
            // G5 A1: 删除真实 USDC 转账（纯记账模式 — USDC 留在主金库）
            // 原: token_compat::transfer(... vault → pm_fee_vault ...)
            // 手续费仅通过 PDA 统计字段累加追踪
            msg!("PM fee {} recorded (pure accounting, no transfer)", fee_amount);
            let _ = pm_fee_vault_info; // suppress unused warning
            
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
            
            let _vault_config_seeds: &[&[u8]] = &[b"vault_config", &[vault_config_bump]];
            
            // G5 A1: 删除真实 USDC 转账（纯记账模式）
            msg!("PM fee {} recorded (pure accounting, no transfer)", fee_amount);
            let _ = pm_fee_vault_info;
            
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
            
            let _vault_config_seeds: &[&[u8]] = &[b"vault_config", &[vault_config_bump]];
            
            // G5 A1: 删除真实 USDC 转账（纯记账模式）
            msg!("PM trading fee {} recorded (pure accounting, no transfer)", fee_amount);
            let _ = pm_fee_vault_info;
            
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
            
            let _vault_config_seeds: &[&[u8]] = &[b"vault_config", &[vault_config_bump]];
            
            // G5 A1: 删除真实 USDC 转账（纯记账模式）
            msg!("PM settlement fee {} recorded (pure accounting, no transfer)", fee_amount);
            let _ = pm_fee_vault_info;
            
            // CRITICAL-3 修复：更新 PM Fee Config 统计（settlement 费用计入 trading_fee 统计）
            const SETTLE_FEE_OFFSET: usize = 73; // 与 TOTAL_TRADING_FEE_OFFSET 相同
            let mut pm_fee_config_data = pm_fee_config_info.try_borrow_mut_data()?;
            let current_total = i64::from_le_bytes(
                pm_fee_config_data[SETTLE_FEE_OFFSET..SETTLE_FEE_OFFSET + 8]
                    .try_into()
                    .unwrap()
            );
            let new_total = current_total.saturating_add(fee_amount as i64);
            pm_fee_config_data[SETTLE_FEE_OFFSET..SETTLE_FEE_OFFSET + 8]
                .copy_from_slice(&new_total.to_le_bytes());
            drop(pm_fee_config_data);
            
            msg!("✅ Settlement fee {} collected + PDA stats updated (total_trading_fee: {})", fee_amount, new_total);
        }

        msg!("✅ PredictionMarketSettleWithFee completed: locked={}, settlement={}, fee={}, net={}", 
             locked_amount, settlement_amount, fee_amount, net_settlement);
        Ok(())
    }

    // =========================================================================
    // SpotTokenBalance PDA helpers (Dynamic Token Balance Architecture)
    // =========================================================================

    /// Verify a SpotTokenBalance PDA address matches the expected derivation.
    /// Returns the bump on success.
    fn verify_spot_balance_pda(
        account_info: &AccountInfo,
        program_id: &Pubkey,
        wallet: &Pubkey,
        token_index: u16,
    ) -> Result<u8, ProgramError> {
        let (expected_pda, bump) = derive_spot_token_balance_pda(program_id, wallet, token_index);
        if account_info.key != &expected_pda {
            msg!("❌ Invalid SpotTokenBalance PDA: expected={}, got={}", expected_pda, account_info.key);
            return Err(VaultError::InvalidPda.into());
        }
        Ok(bump)
    }

    /// Auto-initialize a SpotTokenBalance PDA if it doesn't exist yet.
    /// If the account is empty, creates it with `invoke_signed` and initializes fields.
    /// If it already has data, returns the deserialized balance.
    fn auto_init_spot_balance<'a>(
        payer: &AccountInfo<'a>,
        balance_account: &AccountInfo<'a>,
        system_program: &AccountInfo<'a>,
        program_id: &Pubkey,
        wallet: &Pubkey,
        token_index: u16,
        bump: u8,
    ) -> Result<SpotTokenBalance, ProgramError> {
        if !balance_account.data_is_empty() {
            return deserialize_account::<SpotTokenBalance>(&balance_account.data.borrow())
                .map_err(|_| ProgramError::InvalidAccountData);
        }

        let rent = Rent::get()?;
        let space = SPOT_TOKEN_BALANCE_SIZE;
        let lamports = rent.minimum_balance(space);

        let seeds: &[&[u8]] = &[
            SPOT_BALANCE_SEED,
            wallet.as_ref(),
            &token_index.to_le_bytes(),
            &[bump],
        ];

        if balance_account.lamports() > 0 {
            let required = lamports.saturating_sub(balance_account.lamports());
            if required > 0 {
                invoke(
                    &system_instruction::transfer(payer.key, balance_account.key, required),
                    &[payer.clone(), balance_account.clone(), system_program.clone()],
                )?;
            }
            invoke_signed(
                &system_instruction::allocate(balance_account.key, space as u64),
                &[balance_account.clone(), system_program.clone()],
                &[seeds],
            )?;
            invoke_signed(
                &system_instruction::assign(balance_account.key, program_id),
                &[balance_account.clone(), system_program.clone()],
                &[seeds],
            )?;
        } else {
            invoke_signed(
                &system_instruction::create_account(
                    payer.key,
                    balance_account.key,
                    lamports,
                    space as u64,
                    program_id,
                ),
                &[payer.clone(), balance_account.clone(), system_program.clone()],
                &[seeds],
            )?;
        }

        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        let balance = SpotTokenBalance::new(*wallet, token_index, bump, current_ts);
        balance.serialize(&mut &mut balance_account.data.borrow_mut()[..])?;
        msg!("✅ SpotTokenBalance auto-initialized: wallet={}, token_index={}", wallet, token_index);
        Ok(balance)
    }

    // =========================================================================
    // Spot 交易指令处理 (Dynamic Token Balance Architecture — Plan A)
    // All functions operate on SpotTokenBalance PDAs. No SpotUserAccount.
    // =========================================================================

    /// [DEPRECATED] InitializeSpotUser — returns error
    fn process_initialize_spot_user(
        _program_id: &Pubkey,
        _accounts: &[AccountInfo],
    ) -> ProgramResult {
        msg!("❌ InitializeSpotUser is deprecated. SpotTokenBalance PDAs are auto-initialized on first deposit.");
        Err(VaultError::DeprecatedInstruction.into())
    }

    /// Spot Token 入金 (用户直接调用)
    /// Accounts: user(signer) + balance_pda(w) + user_token + vault_token + vault_config + token_program + system_program
    fn process_spot_deposit(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        // One Account Experience: USDC 必须通过 Vault.Deposit，不能通过 SpotDeposit
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) must use Vault.Deposit, not SpotDeposit. Use Vault instruction #2.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }
        
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let _vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        let bump = Self::verify_spot_balance_pda(balance_pda_info, program_id, user.key, token_index)?;

        let mut balance = Self::auto_init_spot_balance(
            user, balance_pda_info, system_program, program_id, user.key, token_index, bump,
        )?;

        token_compat::transfer(
            token_program, user_token_account, vault_token_account, user, amount, None,
        )?;

        balance.available_e6 = balance.available_e6.checked_add(amount as i64).ok_or(VaultError::Overflow)?;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        msg!("✅ SpotDeposit: token_index={}, amount={}", token_index, amount);
        Ok(())
    }

    /// Spot Token 出金 (用户直接调用)
    /// Accounts: user(signer) + balance_pda(w) + user_token + vault_token + vault_config + token_program
    fn process_spot_withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        // One Account Experience: USDC 必须通过 Vault.Withdraw，不能通过 SpotWithdraw
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) must use Vault.Withdraw, not SpotWithdraw. Use Vault instruction #3.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }
        
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        Self::verify_spot_balance_pda(balance_pda_info, program_id, user.key, token_index)?;

        let mut balance = deserialize_account::<SpotTokenBalance>(&balance_pda_info.data.borrow())?;
        if balance.available_e6 < amount as i64 {
            msg!("❌ Insufficient balance: available={}, required={}", balance.available_e6, amount);
            return Err(VaultError::InsufficientBalance.into());
        }

        balance.available_e6 -= amount as i64;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;

        let (vault_config_pda, vault_config_bump) = Pubkey::find_program_address(&[b"vault_config"], program_id);
        if vault_config_info.key != &vault_config_pda {
            return Err(VaultError::InvalidPda.into());
        }

        token_compat::transfer(
            token_program, vault_token_account, user_token_account, vault_config_info, amount,
            Some(&[b"vault_config", &[vault_config_bump]]),
        )?;

        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;
        msg!("✅ SpotWithdraw: token_index={}, amount={}", token_index, amount);
        Ok(())
    }

    /// Spot 锁定余额 (CPI only)
    fn process_spot_lock_balance(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) cannot use SpotLockBalance. Use SpotLockUsdc instead.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        let mut balance = deserialize_account::<SpotTokenBalance>(&balance_pda_info.data.borrow())?;
        if balance.available_e6 < amount as i64 {
            return Err(VaultError::InsufficientBalance.into());
        }
        balance.available_e6 -= amount as i64;
        balance.locked_e6 = balance.locked_e6.checked_add(amount as i64).ok_or(VaultError::Overflow)?;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        msg!("✅ SpotLockBalance: token_index={}, amount={}", token_index, amount);
        Ok(())
    }

    /// Spot 解锁余额 (CPI only)
    fn process_spot_unlock_balance(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) cannot use SpotUnlockBalance. Use SpotUnlockUsdc instead.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

        let mut balance = deserialize_account::<SpotTokenBalance>(&balance_pda_info.data.borrow())?;
        if balance.locked_e6 < amount as i64 {
            return Err(VaultError::InsufficientBalance.into());
        }
        balance.locked_e6 -= amount as i64;
        balance.available_e6 = balance.available_e6.checked_add(amount as i64).ok_or(VaultError::Overflow)?;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        msg!("✅ SpotUnlockBalance: token_index={}, amount={}", token_index, amount);
        Ok(())
    }

    /// [DEPRECATED] SpotSettleTrade (CPI-only) — use RelayerSpotSettleTrade
    fn process_spot_settle_trade(
        _accounts: &[AccountInfo],
        _is_buy: bool,
        _base_token_index: u16,
        _quote_token_index: u16,
        _base_amount: u64,
        _quote_amount: u64,
        _sequence: u64,
    ) -> ProgramResult {
        msg!("❌ SpotSettleTrade (CPI) is deprecated. Use RelayerSpotSettleTrade.");
        Err(VaultError::DeprecatedInstruction.into())
    }

    /// Relayer 代理 Spot 入金
    /// Accounts: admin(signer) + balance_pda(w) + vault_config + system_program
    fn process_relayer_spot_deposit(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        // One Account Experience: USDC 必须通过 RelayerDeposit，不能通过 RelayerSpotDeposit
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) must use RelayerDeposit (#25), not RelayerSpotDeposit.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }
        
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        let bump = Self::verify_spot_balance_pda(balance_pda_info, program_id, &user_wallet, token_index)?;
        let mut balance = Self::auto_init_spot_balance(
            admin, balance_pda_info, system_program, program_id, &user_wallet, token_index, bump,
        )?;

        balance.available_e6 = balance.available_e6.checked_add(amount as i64).ok_or(VaultError::Overflow)?;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        msg!("✅ RelayerSpotDeposit: user={}, token_index={}, amount={}", user_wallet, token_index, amount);
        Ok(())
    }

    /// Relayer 代理 Spot 出金
    /// Accounts: admin(signer) + balance_pda(w) + vault_config
    fn process_relayer_spot_withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        token_index: u16,
        amount: u64,
    ) -> ProgramResult {
        // One Account Experience: USDC 必须通过 RelayerWithdraw，不能通过 RelayerSpotWithdraw
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) must use RelayerWithdraw (#26), not RelayerSpotWithdraw.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }
        
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        Self::verify_spot_balance_pda(balance_pda_info, program_id, &user_wallet, token_index)?;
        let mut balance = deserialize_account::<SpotTokenBalance>(&balance_pda_info.data.borrow())?;
        if balance.available_e6 < amount as i64 {
            return Err(VaultError::InsufficientBalance.into());
        }
        balance.available_e6 -= amount as i64;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        msg!("✅ RelayerSpotWithdraw: user={}, token_index={}, amount={}", user_wallet, token_index, amount);
        Ok(())
    }

    // =========================================================================
    // Spot 统一账户指令处理 — 4-PDA settle (Dynamic Token Balance Architecture)
    // =========================================================================

    /// Relayer 代理 Spot 交易结算
    /// Accounts: admin(signer) + maker_base(w) + maker_quote(w) + taker_base(w) + taker_quote(w) + vault_config + system_program
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
        _sequence: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let admin = next_account_info(account_info_iter)?;
        let maker_base_info = next_account_info(account_info_iter)?;
        let maker_quote_info = next_account_info(account_info_iter)?;
        let taker_base_info = next_account_info(account_info_iter)?;
        let taker_quote_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(admin)?;
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.admin != *admin.key {
            return Err(VaultError::UnauthorizedAdmin.into());
        }

        if quote_token_index == 0 {
            msg!("❌ USDC (quote_token_index=0) must use SpotSettleUsdcTrade, not RelayerSpotSettleTrade");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        if base_token_index == 0 {
            msg!("❌ USDC (base_token_index=0) cannot be a base asset in Spot trades");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        let maker_base_bump = Self::verify_spot_balance_pda(maker_base_info, program_id, &maker_wallet, base_token_index)?;
        let _maker_quote_bump = Self::verify_spot_balance_pda(maker_quote_info, program_id, &maker_wallet, quote_token_index)?;
        let taker_base_bump = Self::verify_spot_balance_pda(taker_base_info, program_id, &taker_wallet, base_token_index)?;
        let _taker_quote_bump = Self::verify_spot_balance_pda(taker_quote_info, program_id, &taker_wallet, quote_token_index)?;

        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        let is_self_trade = maker_wallet == taker_wallet;

        if is_self_trade {
            // Self-trade: base PDA and quote PDA each have only one copy
            // base: net change = 0 (buy and sell cancel out)
            // quote: deduct total fees (maker_fee + taker_fee)
            let mut quote_bal = deserialize_account::<SpotTokenBalance>(&maker_quote_info.data.borrow())?;
            let total_fee = maker_fee_e6 + taker_fee_e6;
            if total_fee > 0 {
                quote_bal.deduct_prefer_available(total_fee).map_err(|e| {
                    msg!("❌ Self-trade fee deduction failed: {}", e);
                    VaultError::SettlementFailed
                })?;
            }
            quote_bal.last_update_ts = current_ts;
            quote_bal.serialize(&mut &mut maker_quote_info.data.borrow_mut()[..])?;
        } else {
            // Normal path: 4 independent PDAs

            // Auto-init taker_base if buyer gets new token
            if taker_is_buy && taker_base_info.data_is_empty() {
                Self::auto_init_spot_balance(
                    admin, taker_base_info, system_program, program_id,
                    &taker_wallet, base_token_index, taker_base_bump,
                )?;
            }
            // Auto-init maker_base if seller hasn't had this token
            if !taker_is_buy && maker_base_info.data_is_empty() {
                Self::auto_init_spot_balance(
                    admin, maker_base_info, system_program, program_id,
                    &maker_wallet, base_token_index, maker_base_bump,
                )?;
            }

            if taker_is_buy {
                // Taker buys: pays quote + taker_fee, gets base
                // Maker sells: pays base, gets quote - maker_fee

                let mut taker_quote = deserialize_account::<SpotTokenBalance>(&taker_quote_info.data.borrow())?;
                let taker_cost = quote_amount_e6 + taker_fee_e6;
                taker_quote.deduct_prefer_available(taker_cost).map_err(|e| {
                    msg!("❌ Taker quote deduction failed: {}", e);
                    VaultError::SettlementFailed
                })?;
                taker_quote.last_update_ts = current_ts;
                taker_quote.serialize(&mut &mut taker_quote_info.data.borrow_mut()[..])?;

                let mut taker_base = deserialize_account::<SpotTokenBalance>(&taker_base_info.data.borrow())?;
                taker_base.available_e6 = taker_base.available_e6.checked_add(base_amount_e6).ok_or(VaultError::Overflow)?;
                taker_base.last_update_ts = current_ts;
                taker_base.serialize(&mut &mut taker_base_info.data.borrow_mut()[..])?;

                let mut maker_base = deserialize_account::<SpotTokenBalance>(&maker_base_info.data.borrow())?;
                maker_base.deduct_prefer_available(base_amount_e6).map_err(|e| {
                    msg!("❌ Maker base deduction failed: {}", e);
                    VaultError::SettlementFailed
                })?;
                maker_base.last_update_ts = current_ts;
                maker_base.serialize(&mut &mut maker_base_info.data.borrow_mut()[..])?;

                let mut maker_quote = deserialize_account::<SpotTokenBalance>(&maker_quote_info.data.borrow())?;
                let maker_receives = quote_amount_e6 - maker_fee_e6;
                maker_quote.available_e6 = maker_quote.available_e6.checked_add(maker_receives).ok_or(VaultError::Overflow)?;
                maker_quote.last_update_ts = current_ts;
                maker_quote.serialize(&mut &mut maker_quote_info.data.borrow_mut()[..])?;
            } else {
                // Taker sells: pays base, gets quote - taker_fee
                // Maker buys: pays quote + maker_fee, gets base

                let mut taker_base = deserialize_account::<SpotTokenBalance>(&taker_base_info.data.borrow())?;
                taker_base.deduct_prefer_available(base_amount_e6).map_err(|e| {
                    msg!("❌ Taker base deduction failed: {}", e);
                    VaultError::SettlementFailed
                })?;
                taker_base.last_update_ts = current_ts;
                taker_base.serialize(&mut &mut taker_base_info.data.borrow_mut()[..])?;

                let mut taker_quote = deserialize_account::<SpotTokenBalance>(&taker_quote_info.data.borrow())?;
                let taker_receives = quote_amount_e6 - taker_fee_e6;
                if taker_receives < 0 {
                    msg!("❌ Taker fee exceeds quote amount");
                    return Err(VaultError::SettlementFailed.into());
                }
                taker_quote.available_e6 = taker_quote.available_e6.checked_add(taker_receives).ok_or(VaultError::Overflow)?;
                taker_quote.last_update_ts = current_ts;
                taker_quote.serialize(&mut &mut taker_quote_info.data.borrow_mut()[..])?;

                let mut maker_quote = deserialize_account::<SpotTokenBalance>(&maker_quote_info.data.borrow())?;
                let maker_cost = quote_amount_e6 + maker_fee_e6;
                maker_quote.deduct_prefer_available(maker_cost).map_err(|e| {
                    msg!("❌ Maker quote deduction failed: {}", e);
                    VaultError::SettlementFailed
                })?;
                maker_quote.last_update_ts = current_ts;
                maker_quote.serialize(&mut &mut maker_quote_info.data.borrow_mut()[..])?;

                let mut maker_base = deserialize_account::<SpotTokenBalance>(&maker_base_info.data.borrow())?;
                maker_base.available_e6 = maker_base.available_e6.checked_add(base_amount_e6).ok_or(VaultError::Overflow)?;
                maker_base.last_update_ts = current_ts;
                maker_base.serialize(&mut &mut maker_base_info.data.borrow_mut()[..])?;
            }
        }

        msg!("✅ RelayerSpotSettleTrade: maker={}, taker={}, base={}, quote={}, self_trade={}",
             maker_wallet, taker_wallet, base_amount_e6, quote_amount_e6, is_self_trade);
        Ok(())
    }

    /// [DEPRECATED] 从 UserAccount 划转 USDC 到 SpotTokenBalance
    /// 
    /// One Account Experience: 此指令已废弃。
    /// USDC 现在通过 SpotLockUsdc/SpotUnlockUsdc 在 UserAccount 内部管理。
    /// 不再需要将 USDC 搬运到 SpotTokenBalance PDA。
    fn process_spot_allocate_from_vault(
        _program_id: &Pubkey,
        _accounts: &[AccountInfo],
        _user_wallet: Pubkey,
        _amount: u64,
    ) -> ProgramResult {
        msg!("❌ SpotAllocateFromVault is deprecated. Use SpotLockUsdc instead (One Account Experience).");
        Err(VaultError::DeprecatedInstruction.into())
    }

    /// [DEPRECATED] 从 SpotTokenBalance 划转 USDC 到 UserAccount
    /// 
    /// One Account Experience: 此指令已废弃。
    /// USDC 现在通过 SpotLockUsdc/SpotUnlockUsdc 在 UserAccount 内部管理。
    /// Spot 卖出获得的 USDC 直接 credit 到 seller 的 UserAccount.available。
    fn process_spot_release_to_vault(
        _program_id: &Pubkey,
        _accounts: &[AccountInfo],
        _user_wallet: Pubkey,
        _amount: u64,
    ) -> ProgramResult {
        msg!("❌ SpotReleaseToVault is deprecated. USDC now stays in UserAccount (One Account Experience).");
        Err(VaultError::DeprecatedInstruction.into())
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

    /// G5 A3: CreditUserBalance — 纯记账余额增加
    ///
    /// 仅限 Fund Program 通过 CPI 调用。
    /// 用于手续费分配、管理费收取等场景，替代真实 SPL Token Transfer。
    ///
    /// Accounts:
    /// 0. `[signer]` Caller Program PDA（Fund Program 签名）
    /// 1. `[]` VaultConfig（验证 caller 是 Fund Program）
    /// 2. `[writable]` UserAccount PDA（目标用户）
    fn process_credit_user_balance(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let caller_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;

        // 1. 验证 caller 签名
        if !caller_info.is_signer {
            msg!("❌ CreditUserBalance: caller must sign");
            return Err(VaultError::MissingSignature.into());
        }

        // 2. 验证 VaultConfig 并确认 caller 是 Fund Program
        let vault_config: VaultConfig = deserialize_account(&vault_config_info.data.borrow())?;
        if vault_config.fund_program != *caller_info.key {
            msg!("❌ CreditUserBalance: caller {} is not Fund Program {}", 
                caller_info.key, vault_config.fund_program);
            return Err(VaultError::UnauthorizedCaller.into());
        }

        // 3. M2 安全加固：验证 UserAccount PDA 对应 user_wallet
        assert_writable(user_account_info)?;
        let (expected_user_pda, _) = Pubkey::find_program_address(
            &[b"user", user_wallet.as_ref()],
            _program_id,
        );
        if user_account_info.key != &expected_user_pda {
            msg!("❌ CreditUserBalance: UserAccount PDA mismatch. Expected {} for wallet {}", expected_user_pda, user_wallet);
            return Err(VaultError::InvalidPda.into());
        }

        // 4. 更新余额
        let mut user_account: UserAccount = deserialize_account(&user_account_info.data.borrow())?;
        user_account.available_balance_e6 = checked_add(
            user_account.available_balance_e6, 
            amount as i64,
        )?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!(
            "✅ CreditUserBalance: wallet={}, amount={}, new_balance={}",
            user_wallet, amount, user_account.available_balance_e6
        );
        Ok(())
    }

    // =========================================================================
    // One Account Experience — Spot USDC 统一管理
    // =========================================================================

    /// SpotLockUsdc — 将 USDC 从 available 移到 spot_locked（同一 PDA 内）
    /// 
    /// 与 LockMargin (Perp) 完全对称。
    /// Accounts: VaultConfig + UserAccount(w) + Admin(signer) or CallerProgram(CPI)
    fn process_spot_lock_usdc(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        if amount > i64::MAX as u64 {
            msg!("❌ SpotLockUsdc: amount {} exceeds i64::MAX", amount);
            return Err(VaultError::Overflow.into());
        }

        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let caller = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.is_paused {
            return Err(VaultError::VaultPaused.into());
        }
        verify_admin_or_cpi_caller(&vault_config, caller)?;

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        if user_account.available_balance_e6 < amount as i64 {
            msg!("❌ SpotLockUsdc: insufficient available={}, required={}", 
                user_account.available_balance_e6, amount);
            return Err(VaultError::InsufficientBalance.into());
        }

        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, amount as i64)?;
        user_account.spot_locked_e6 = checked_add(user_account.spot_locked_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotLockUsdc: {} e6 locked for {}", amount, user_account.wallet);
        Ok(())
    }

    /// SpotUnlockUsdc — 将 USDC 从 spot_locked 移回 available
    /// 
    /// 与 ReleaseMargin (Perp) 完全对称。用于撤单或回滚。
    /// Accounts: VaultConfig + UserAccount(w) + Admin(signer) or CallerProgram(CPI)
    fn process_spot_unlock_usdc(accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        if amount > i64::MAX as u64 {
            msg!("❌ SpotUnlockUsdc: amount {} exceeds i64::MAX", amount);
            return Err(VaultError::Overflow.into());
        }

        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let caller = next_account_info(account_info_iter)?;

        assert_writable(user_account_info)?;

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_admin_or_cpi_caller(&vault_config, caller)?;

        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
        if user_account.spot_locked_e6 < amount as i64 {
            msg!("❌ SpotUnlockUsdc: insufficient spot_locked={}, required={}", 
                user_account.spot_locked_e6, amount);
            return Err(VaultError::InsufficientSpotLocked.into());
        }

        user_account.spot_locked_e6 = checked_sub(user_account.spot_locked_e6, amount as i64)?;
        user_account.available_balance_e6 = checked_add(user_account.available_balance_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("✅ SpotUnlockUsdc: {} e6 unlocked for {}", amount, user_account.wallet);
        Ok(())
    }

    /// SpotSettleUsdcTrade — 原子结算 Spot 交易的 USDC + base token
    /// 
    /// 一笔交易同时操作 4 个 PDA：
    ///   Buyer UserAccount:         spot_locked -= (buyer_usdc + buyer_fee)
    ///   Seller UserAccount:        available += (seller_credit - seller_fee)
    ///   Buyer SpotTokenBalance:    available += base_amount (auto-init)
    ///   Seller SpotTokenBalance:   locked -= base_amount
    ///
    /// 自交处理：当 buyer == seller 时合并为一次 UserAccount 读写。
    fn process_spot_settle_usdc_trade(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        buyer_usdc: u64,
        seller_credit: u64,
        buyer_fee: u64,
        seller_fee: u64,
        base_amount: u64,
        sequence: u64,
        base_token_index: u16,
    ) -> ProgramResult {
        if buyer_usdc > i64::MAX as u64 || seller_credit > i64::MAX as u64
            || buyer_fee > i64::MAX as u64 || seller_fee > i64::MAX as u64
            || base_amount > i64::MAX as u64 {
            msg!("❌ SpotSettleUsdcTrade: amount exceeds i64::MAX");
            return Err(VaultError::Overflow.into());
        }

        if base_token_index == 0 {
            msg!("❌ SpotSettleUsdcTrade: USDC (base_token_index=0) cannot be a base asset");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let buyer_account_info = next_account_info(account_info_iter)?;
        let seller_account_info = next_account_info(account_info_iter)?;
        let buyer_base_info = next_account_info(account_info_iter)?;
        let seller_base_info = next_account_info(account_info_iter)?;
        let caller = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_writable(buyer_account_info)?;
        assert_writable(seller_account_info)?;
        assert_writable(buyer_base_info)?;
        assert_writable(seller_base_info)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.is_paused {
            return Err(VaultError::VaultPaused.into());
        }
        verify_admin_or_cpi_caller(&vault_config, caller)?;

        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        
        // 检测自交
        let is_self_trade = buyer_account_info.key == seller_account_info.key;

        if is_self_trade {
            // --- 自交处理：同一个 UserAccount，base token net = 0 ---
            let mut user = deserialize_account::<UserAccount>(&buyer_account_info.data.borrow())?;
            
            // USDC: spot_locked -= (buyer_usdc + buyer_fee), available += (seller_credit - seller_fee)
            let total_buyer_deduct = checked_add(buyer_usdc as i64, buyer_fee as i64)?;
            if user.spot_locked_e6 < total_buyer_deduct {
                msg!("❌ SpotSettleUsdcTrade self-trade: insufficient spot_locked={}, required={}", 
                    user.spot_locked_e6, total_buyer_deduct);
                return Err(VaultError::InsufficientSpotLocked.into());
            }
            user.spot_locked_e6 = checked_sub(user.spot_locked_e6, total_buyer_deduct)?;
            
            let seller_net = checked_sub(seller_credit as i64, seller_fee as i64)?;
            user.available_balance_e6 = checked_add(user.available_balance_e6, seller_net)?;
            user.last_update_ts = current_ts;
            user.serialize(&mut &mut buyer_account_info.data.borrow_mut()[..])?;
            
            // Base token: self-trade — total unchanged, but must migrate locked → available
            if base_amount > 0 {
                let mut base = deserialize_account::<SpotTokenBalance>(&buyer_base_info.data.borrow())?;
                if base.locked_e6 >= base_amount as i64 {
                    base.locked_e6 = checked_sub(base.locked_e6, base_amount as i64)?;
                    base.available_e6 = checked_add(base.available_e6, base_amount as i64)?;
                    base.last_update_ts = current_ts;
                    base.serialize(&mut &mut buyer_base_info.data.borrow_mut()[..])?;
                }
            }
            
            msg!("✅ SpotSettleUsdcTrade (self-trade): seq={}, usdc_fee={}, base_migrated={}, wallet={}",
                sequence, buyer_fee + seller_fee, base_amount, user.wallet);
        } else {
            // --- 非自交：分别更新 buyer 和 seller ---
            
            // Buyer: spot_locked -= (buyer_usdc + buyer_fee)
            let mut buyer = deserialize_account::<UserAccount>(&buyer_account_info.data.borrow())?;
            let total_buyer_deduct = checked_add(buyer_usdc as i64, buyer_fee as i64)?;
            if buyer.spot_locked_e6 < total_buyer_deduct {
                msg!("❌ SpotSettleUsdcTrade: buyer insufficient spot_locked={}, required={}", 
                    buyer.spot_locked_e6, total_buyer_deduct);
                return Err(VaultError::InsufficientSpotLocked.into());
            }
            buyer.spot_locked_e6 = checked_sub(buyer.spot_locked_e6, total_buyer_deduct)?;
            buyer.last_update_ts = current_ts;
            buyer.serialize(&mut &mut buyer_account_info.data.borrow_mut()[..])?;

            // Seller: available += (seller_credit - seller_fee)
            let mut seller = deserialize_account::<UserAccount>(&seller_account_info.data.borrow())?;
            let seller_net = checked_sub(seller_credit as i64, seller_fee as i64)?;
            seller.available_balance_e6 = checked_add(seller.available_balance_e6, seller_net)?;
            seller.last_update_ts = current_ts;
            seller.serialize(&mut &mut seller_account_info.data.borrow_mut()[..])?;

            // Buyer base token: available += base_amount (auto-init if needed)
            let buyer_wallet = buyer.wallet;
            if buyer_base_info.data_is_empty() {
                let buyer_base_bump = Self::verify_spot_balance_pda(
                    buyer_base_info, program_id, &buyer_wallet, base_token_index
                )?;
                Self::auto_init_spot_balance(
                    caller, buyer_base_info, system_program, program_id,
                    &buyer_wallet, base_token_index, buyer_base_bump,
                )?;
            }
            let mut buyer_base = deserialize_account::<SpotTokenBalance>(&buyer_base_info.data.borrow())?;
            buyer_base.available_e6 = checked_add(buyer_base.available_e6, base_amount as i64)?;
            buyer_base.last_update_ts = current_ts;
            buyer_base.serialize(&mut &mut buyer_base_info.data.borrow_mut()[..])?;

            // Seller base token: locked -= base_amount
            let mut seller_base = deserialize_account::<SpotTokenBalance>(&seller_base_info.data.borrow())?;
            if seller_base.locked_e6 < base_amount as i64 {
                msg!("❌ SpotSettleUsdcTrade: seller insufficient base locked={}, required={}", 
                    seller_base.locked_e6, base_amount);
                return Err(VaultError::InsufficientBalance.into());
            }
            seller_base.locked_e6 = checked_sub(seller_base.locked_e6, base_amount as i64)?;
            seller_base.last_update_ts = current_ts;
            seller_base.serialize(&mut &mut seller_base_info.data.borrow_mut()[..])?;

            msg!("✅ SpotSettleUsdcTrade: seq={}, buyer={}, seller={}, usdc={}, base={}",
                sequence, buyer_wallet, seller.wallet, buyer_usdc, base_amount);
        }

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
