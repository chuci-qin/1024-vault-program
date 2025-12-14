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
                Self::process_prediction_market_settle(accounts, locked_amount, settlement_amount)
            }
            VaultInstruction::PredictionMarketClaimSettlement => {
                msg!("Instruction: PredictionMarketClaimSettlement");
                Self::process_prediction_market_claim_settlement(accounts)
            }
            VaultInstruction::AdminPredictionMarketForceUnlock { amount } => {
                msg!("Instruction: AdminPredictionMarketForceUnlock");
                Self::process_admin_prediction_market_force_unlock(accounts, amount)
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

        // SPL Token Transfer (用户 → Vault)
        let transfer_ix = spl_token::instruction::transfer(
            token_program.key,
            user_token_account.key,
            vault_token_account.key,
            user.key,
            &[],
            amount,
        )?;

        invoke(
            &transfer_ix,
            &[
                user_token_account.clone(),
                vault_token_account.clone(),
                user.clone(),
                token_program.clone(),
            ],
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

        // SPL Token Transfer (Vault → 用户) - 使用PDA签名
        let (vault_config_pda, vault_config_bump) =
            Pubkey::find_program_address(&[b"vault_config"], vault_config_info.owner);

        let transfer_ix = spl_token::instruction::transfer(
            token_program.key,
            vault_token_account.key,
            user_token_account.key,
            &vault_config_pda,
            &[],
            amount,
        )?;

        invoke_signed(
            &transfer_ix,
            &[
                vault_token_account.clone(),
                user_token_account.clone(),
                vault_config_info.clone(),
                token_program.clone(),
            ],
            &[&[b"vault_config", &[vault_config_bump]]],
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
            
            let transfer_ix = spl_token::instruction::transfer(
                &spl_token::id(),
                vault_token_account.key,
                insurance_fund_vault.key,
                vault_config_info.key, // VaultConfig PDA is the authority
                &[],
                liquidation_penalty,
            )?;
            
            invoke_signed(
                &transfer_ix,
                &[
                    vault_token_account.clone(),
                    insurance_fund_vault.clone(),
                    vault_config_info.clone(),
                    token_program.clone(),
                ],
                &[&[b"vault_config", &[bump]]],
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
    fn process_prediction_market_settle(
        accounts: &[AccountInfo],
        locked_amount: u64,
        settlement_amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let vault_config_info = next_account_info(account_info_iter)?;
        let pm_user_account_info = next_account_info(account_info_iter)?;
        let caller_program = next_account_info(account_info_iter)?;

        assert_writable(pm_user_account_info)?;

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        verify_cpi_caller(&vault_config, caller_program)?;

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
}

/// Program entrypoint's implementation
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    Processor::process(program_id, accounts, instruction_data)
}
