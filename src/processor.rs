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
    } else if vault_config.authorized_callers.contains(caller_program.key) {
        msg!("✅ CPI caller in authorized list");
    } else if vault_config.fund_program.as_ref() == Some(caller_program.key) {
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
            authorized_callers: Vec::new(),
            ledger_program,
            fund_program: Some(fund_program),
            delegation_program,
            total_deposits: 0,
            total_locked: 0,
            is_paused: false,
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

        if !vault_config.authorized_callers.contains(&caller) {
            vault_config.authorized_callers.push(caller);
            vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;
            msg!("Added authorized caller: {}", caller);
        } else {
            msg!("Caller already authorized: {}", caller);
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

        vault_config.authorized_callers.retain(|&x| x != caller);
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Removed authorized caller: {}", caller);
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

        vault_config.fund_program = Some(fund_program);
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Fund program set to: {}", fund_program);
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
