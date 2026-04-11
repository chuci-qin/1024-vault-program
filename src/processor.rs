//! Vault Program Processor
//!
//! 职责: 用户资金托管 — DB-First + 实时链上审计架构中的链上 Vault 组件
//!
//! ## 功能域 (18 active instructions)
//!
//! | # | 域 | Handler 范围 | 说明 |
//! |---|------|-------------|------|
//! | 1 | Core VaultSettlement | `process_initialize` ~ `process_withdraw` | 初始化、用户入金/出金 |
//! | 2 | Relayer VaultSettlement | `process_relayer_deposit` ~ `process_relayer_withdraw_and_transfer` | 代理入金/出金（含跨链提取） |
//! | 3 | Spot VaultSettlement | `process_spot_deposit` ~ `process_relayer_spot_withdraw` | Spot 资产入金/出金 |
//! | 4 | Mirror | `process_user_account` ~ `process_spot_token_balance` | 链上 PDA 状态镜像 |
//! | 5 | Governance Authority | `process_add_authorized_caller` ~ `process_migrate_vault_config` | 配置管理、升级迁移 |
//!
//! ## 架构要点
//!
//! - 只有托管出入金路径涉及真实 SPL Token 转账（用户/Relayer）
//! - 链上仅保留托管与审计镜像；业务结算与资金计算在 DB 内完成
//! - 镜像指令幂等地将状态写入链上

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

/// 辅助函数：反序列化账户数据
fn deserialize_account<T: BorshDeserialize>(data: &[u8]) -> Result<T, std::io::Error> {
    let mut slice = data;
    T::deserialize(&mut slice)
}

/// OC-M2: Deserialize + discriminator check for Vault PDA accounts.
/// All Vault structs have `discriminator: u64` as the first 8 bytes.
fn deserialize_checked(data: &[u8], expected_discriminator: u64) -> Result<(), ProgramError> {
    if data.len() < 8 {
        return Err(ProgramError::InvalidAccountData);
    }
    let disc = u64::from_le_bytes(data[..8].try_into().map_err(|_| ProgramError::InvalidAccountData)?);
    if disc != expected_discriminator {
        msg!("❌ Discriminator mismatch: expected 0x{:016X}, got 0x{:016X}", expected_discriminator, disc);
        return Err(ProgramError::InvalidAccountData);
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
            VaultInstruction::Initialize { delegation_program } => {
                msg!("Instruction: Initialize");
                Self::process_initialize(program_id, accounts, delegation_program)
            }
            VaultInstruction::InitializeUser { account_index } => {
                msg!("Instruction: InitializeUser");
                Self::process_initialize_user(program_id, accounts, account_index)
            }
            VaultInstruction::Deposit { amount } => {
                msg!("Instruction: Deposit");
                Self::process_deposit(program_id, accounts, amount)
            }
            VaultInstruction::Withdraw { amount } => {
                msg!("Instruction: Withdraw");
                Self::process_withdraw(program_id, accounts, amount)
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
            VaultInstruction::UpdateGovernanceAuthority { new_governance_authority } => {
                msg!("Instruction: UpdateGovernanceAuthority");
                Self::process_update_governance_authority(accounts, new_governance_authority)
            }
            VaultInstruction::RelayerDeposit { user_wallet, amount, account_index } => {
                msg!("Instruction: RelayerDeposit");
                Self::process_relayer_deposit(program_id, accounts, user_wallet, amount, account_index)
            }
            VaultInstruction::RelayerWithdraw { user_wallet, amount, account_index } => {
                msg!("Instruction: RelayerWithdraw");
                Self::process_relayer_withdraw(program_id, accounts, user_wallet, amount, account_index)
            }
            VaultInstruction::SpotDeposit { token_index, amount, account_index, amount_e6 } => {
                msg!("Instruction: SpotDeposit");
                Self::process_spot_deposit(program_id, accounts, token_index, amount, account_index, amount_e6)
            }
            VaultInstruction::SpotWithdraw { token_index, amount, account_index, amount_e6 } => {
                msg!("Instruction: SpotWithdraw");
                Self::process_spot_withdraw(program_id, accounts, token_index, amount, account_index, amount_e6)
            }
            VaultInstruction::RelayerSpotDeposit { user_wallet, token_index, amount, account_index, amount_e6 } => {
                msg!("Instruction: RelayerSpotDeposit");
                Self::process_relayer_spot_deposit(program_id, accounts, user_wallet, token_index, amount, account_index, amount_e6)
            }
            VaultInstruction::RelayerSpotWithdraw { user_wallet, token_index, amount, account_index, amount_e6 } => {
                msg!("Instruction: RelayerSpotWithdraw");
                Self::process_relayer_spot_withdraw(program_id, accounts, user_wallet, token_index, amount, account_index, amount_e6)
            }
            VaultInstruction::RelayerWithdrawAndTransfer { user_wallet, amount, account_index } => {
                msg!("Instruction: RelayerWithdrawAndTransfer");
                Self::process_relayer_withdraw_and_transfer(program_id, accounts, user_wallet, amount, account_index)
            }
            VaultInstruction::UserAccount { user_wallet, account_index, available_balance_e6, locked_margin_e6, spot_locked_e6, oracle_locked_e6 } => {
                msg!("Instruction: UserAccount");
                Self::process_user_account(program_id, accounts, user_wallet, account_index, available_balance_e6, locked_margin_e6, spot_locked_e6, oracle_locked_e6)
            }
            VaultInstruction::SpotTokenBalance { user_wallet, account_index, token_index, available_e6, locked_e6 } => {
                msg!("Instruction: SpotTokenBalance");
                Self::process_spot_token_balance(program_id, accounts, user_wallet, account_index, token_index, available_e6, locked_e6)
            }
            VaultInstruction::MigrateVaultConfig => {
                msg!("Instruction: MigrateVaultConfig (V1 569→V2 505)");
                Self::process_migrate_vault_config(program_id, accounts)
            }
        }
    }

    // =========================================================================
    // Core: 初始化、入金、出金
    // =========================================================================

    /// 处理初始化
    fn process_initialize(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        delegation_program: Pubkey,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let usdc_mint = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let _system_program = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;

        let (vault_config_pda, vault_config_bump) =
            Pubkey::find_program_address(&[b"vault_config"], program_id);

        if vault_config_info.key != &vault_config_pda {
            return Err(VaultError::InvalidPda.into());
        }

        let rent = Rent::get()?;
        let space = VAULT_CONFIG_SIZE;
        let lamports = rent.minimum_balance(space);

        if !vault_config_info.data_is_empty() || vault_config_info.lamports() > 0 {
            msg!("VaultConfig already initialized");
            return Err(VaultError::AlreadyInitialized.into());
        }

        invoke_signed(
            &system_instruction::create_account(
                governance_authority.key,
                vault_config_info.key,
                lamports,
                space as u64,
                program_id,
            ),
            &[governance_authority.clone(), vault_config_info.clone()],
            &[&[b"vault_config", &[vault_config_bump]]],
        )?;

        let vault_config = VaultConfig {
            discriminator: VaultConfig::DISCRIMINATOR,
            governance_authority: *governance_authority.key,
            usdc_mint: *usdc_mint.key,
            vault_token_account: *vault_token_account.key,
            authorized_callers: [Pubkey::default(); 10],
            delegation_program,
            total_deposits: 0,
            total_locked: 0,
            is_paused: false,
            reserved: [0u8; 32],
        };

        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Vault initialized");
        msg!("Delegation Program: {}", delegation_program);
        Ok(())
    }

    /// 处理初始化用户账户
    fn process_initialize_user(program_id: &Pubkey, accounts: &[AccountInfo], account_index: u8) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let _system_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        let (user_account_pda, bump) = UserAccount::derive_pda(program_id, user.key, account_index);

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
            &[&[b"user", user.key.as_ref(), &[account_index], &[bump]]],
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
            account_index,
            oracle_locked_e6: 0,
            reserved: [0; 47],
        };

        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        msg!("User account initialized for {}", user.key);
        Ok(())
    }

    /// 处理入金（V1-V4: PDA + token_account + vault_config 验证）
    fn process_deposit(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
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

        // V-1: Verify token_program is a known SPL Token program
        if !token_compat::is_valid_token_program(token_program.key) {
            msg!("❌ Invalid token program: expected SPL Token or Token-2022");
            return Err(VaultError::InvalidAccount.into());
        }

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // V-3: Verify VaultConfig PDA
        let (expected_vault_config_pda, _) = Pubkey::find_program_address(&[b"vault_config"], program_id);
        if vault_config_info.key != &expected_vault_config_pda {
            msg!("❌ Invalid VaultConfig PDA");
            return Err(VaultError::InvalidAccount.into());
        }

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.is_paused {
            return Err(VaultError::VaultPaused.into());
        }

        // V-2: Verify vault_token_account matches VaultConfig
        if vault_token_account.key != &vault_config.vault_token_account {
            msg!("❌ Invalid vault token account");
            return Err(VaultError::InvalidAccount.into());
        }

        // OC-H1: Verify user token account mint matches VaultConfig.usdc_mint
        {
            let user_ta_data = user_token_account.data.borrow();
            if user_ta_data.len() < 40 {
                msg!("❌ User token account data too short ({} bytes), expected >= 40", user_ta_data.len());
                return Err(VaultError::InvalidAccount.into());
            }
            let mint_bytes: [u8; 32] = user_ta_data[..32].try_into().unwrap_or([0u8; 32]);
            let user_mint = Pubkey::new_from_array(mint_bytes);
            if user_mint != vault_config.usdc_mint {
                msg!("❌ User token account mint mismatch: expected {}, got {}", vault_config.usdc_mint, user_mint);
                return Err(VaultError::InvalidAccount.into());
            }
        }

        // V-1: Verify UserAccount PDA
        let user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        let (expected_user_pda, _) = UserAccount::derive_pda(program_id, user.key, user_account.account_index);
        if user_account_info.key != &expected_user_pda {
            msg!("❌ Invalid UserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }
        drop(user_account);

        // SPL Token Transfer (用户 → Vault)
        token_compat::transfer(
            token_program,
            user_token_account,
            vault_token_account,
            user,
            amount,
            None,
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

    /// 处理出金（V1-V4: PDA + token_account + vault_config 验证）
    fn process_withdraw(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;
        assert_writable(user_account_info)?;

        // V-1: Verify token_program is a known SPL Token program
        if !token_compat::is_valid_token_program(token_program.key) {
            msg!("❌ Invalid token program: expected SPL Token or Token-2022");
            return Err(VaultError::InvalidAccount.into());
        }

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // V-3: Verify VaultConfig PDA
        let (expected_vault_config_pda, vault_config_bump) =
            Pubkey::find_program_address(&[b"vault_config"], program_id);
        if vault_config_info.key != &expected_vault_config_pda {
            msg!("❌ Invalid VaultConfig PDA");
            return Err(VaultError::InvalidAccount.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.is_paused {
            return Err(VaultError::VaultPaused.into());
        }

        // V-2: Verify vault_token_account matches VaultConfig
        if vault_token_account.key != &vault_config.vault_token_account {
            msg!("❌ Invalid vault token account");
            return Err(VaultError::InvalidAccount.into());
        }

        // OC-H1: Verify user token account mint matches VaultConfig.usdc_mint
        {
            let user_ta_data = user_token_account.data.borrow();
            if user_ta_data.len() < 40 {
                msg!("❌ Withdraw: user token account data too short ({} bytes), expected >= 40", user_ta_data.len());
                return Err(VaultError::InvalidAccount.into());
            }
            let mint_bytes: [u8; 32] = user_ta_data[..32].try_into().unwrap_or([0u8; 32]);
            let user_mint = Pubkey::new_from_array(mint_bytes);
            if user_mint != vault_config.usdc_mint {
                msg!("❌ Withdraw: user token account mint mismatch: expected {}, got {}", vault_config.usdc_mint, user_mint);
                return Err(VaultError::InvalidAccount.into());
            }
        }

        // V-1: Verify UserAccount PDA + OC-M2 discriminator
        deserialize_checked(&user_account_info.data.borrow(), UserAccount::DISCRIMINATOR)?;
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        let (expected_user_pda, _) = UserAccount::derive_pda(program_id, user.key, user_account.account_index);
        if user_account_info.key != &expected_user_pda {
            msg!("❌ Invalid UserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        if user_account.available_balance_e6 < amount as i64 {
            return Err(VaultError::InsufficientBalance.into());
        }

        user_account.available_balance_e6 = checked_sub(user_account.available_balance_e6, amount as i64)?;
        user_account.total_withdrawn_e6 = checked_add(user_account.total_withdrawn_e6, amount as i64)?;
        user_account.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;

        // SPL Token Transfer (Vault → 用户)
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

    // =========================================================================
    // Governance Authority: 权限管理、暂停、紧急释放
    // =========================================================================

    fn process_add_authorized_caller(accounts: &[AccountInfo], caller: Pubkey) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.governance_authority != *governance_authority.key {
            return Err(VaultError::InvalidGovernanceAuthority.into());
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
        let governance_authority = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.governance_authority != *governance_authority.key {
            return Err(VaultError::InvalidGovernanceAuthority.into());
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
            msg!("❌ Caller not found in authorized list: {}", caller);
            return Err(VaultError::UnauthorizedUser.into());
        }

        Ok(())
    }

    fn process_set_paused(accounts: &[AccountInfo], paused: bool) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.governance_authority != *governance_authority.key {
            return Err(VaultError::InvalidGovernanceAuthority.into());
        }

        vault_config.is_paused = paused;
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Vault {}", if paused { "paused" } else { "resumed" });
        Ok(())
    }

    fn process_update_governance_authority(accounts: &[AccountInfo], new_governance_authority: Pubkey) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let current_governance_authority = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(current_governance_authority)?;
        assert_writable(vault_config_info)?;

        let mut vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        
        if vault_config.governance_authority != *current_governance_authority.key {
            return Err(VaultError::InvalidGovernanceAuthority.into());
        }

        vault_config.governance_authority = new_governance_authority;
        vault_config.serialize(&mut &mut vault_config_info.data.borrow_mut()[..])?;

        msg!("Governance authority updated to: {}", new_governance_authority);
        Ok(())
    }
    // =========================================================================
    // Relayer 指令实现
    // =========================================================================

    /// Relayer 代理入金
    /// 
    /// 功能：
    /// 1. 验证 Governance Authority 签名
    /// 2. 如果 UserAccount 不存在，自动创建
    /// 3. 增加用户余额
    /// 
    /// 测试网特性：Governance Authority 可自由给任何用户入金（凭证模式）
    /// OC-L5: RelayerDeposit intentionally skips `is_paused` check.
    /// Rationale: When the vault is paused (e.g. during incident response), cross-chain bridge
    /// deposits must still be processed to avoid stuck user funds on the source chain.
    /// The pause only affects user-initiated Deposit/Withdraw (which require user signature).
    /// Relayer operations (governed by governance_authority) bypass pause by design.
    fn process_relayer_deposit(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        amount: u64,
        account_index: u8,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        // 1. 验证 governance authority 签名和账户可写
        assert_signer(governance_authority)?;
        assert_writable(user_account_info)?;
        // VaultConfig 不需要写入 (不更新 total_deposits)

        // OC-H2: Verify signer is governance_authority or authorized_caller
        let vault_config_data = vault_config_info.data.borrow();
        if !VaultConfig::is_valid_relayer_from_bytes(&vault_config_data, governance_authority.key) {
            msg!("❌ Invalid relayer: {} (not governance_authority nor authorized_caller)", governance_authority.key);
            return Err(VaultError::InvalidRelayer.into());
        }

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // V-6: Per-relayer daily deposit rate limit.
        // The on-chain program cannot query clock-based daily aggregates efficiently,
        // so we enforce a per-TX ceiling here.  The backend (gateway) should enforce
        // the aggregate daily limit before calling this instruction.
        // Max single deposit: 10M USDC (prevents fat-finger or exploit in a single TX).
        const MAX_SINGLE_DEPOSIT_E6: u64 = 10_000_000_000_000; // $10M
        if amount > MAX_SINGLE_DEPOSIT_E6 {
            msg!("❌ V-6: Deposit amount {} exceeds per-TX limit {}", amount, MAX_SINGLE_DEPOSIT_E6);
            return Err(VaultError::InvalidAmount.into());
        }

        // 3. 验证 UserAccount PDA
        let (user_account_pda, bump) = UserAccount::derive_pda(program_id, &user_wallet, account_index);
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
                    governance_authority.key,
                    user_account_info.key,
                    lamports,
                    space as u64,
                    program_id,
                ),
                &[governance_authority.clone(), user_account_info.clone(), system_program.clone()],
                &[&[b"user", user_wallet.as_ref(), &[account_index], &[bump]]],
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
                account_index,
                oracle_locked_e6: 0,
                reserved: [0; 47],
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
    /// 1. 验证 Governance Authority 签名
    /// 2. 验证用户余额充足
    /// 3. 扣除用户余额
    /// 
    /// OC-L5: RelayerWithdraw intentionally skips `is_paused` check.
    /// Rationale: Same as RelayerDeposit — governance-authorized operations bypass pause.
    /// During pause, the bridge relayer must still be able to execute pending withdrawals
    /// that have already been committed on the source chain.
    fn process_relayer_withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        amount: u64,
        account_index: u8,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        // 1. 验证 governance authority 签名和账户可写
        assert_signer(governance_authority)?;
        assert_writable(user_account_info)?;

        // OC-H2: Verify signer is governance_authority or authorized_caller
        let vault_config_data = vault_config_info.data.borrow();
        if !VaultConfig::is_valid_relayer_from_bytes(&vault_config_data, governance_authority.key) {
            msg!("❌ Invalid relayer: {} (not governance_authority nor authorized_caller)", governance_authority.key);
            return Err(VaultError::InvalidRelayer.into());
        }

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // 3. 验证 UserAccount PDA
        let (user_account_pda, _bump) = UserAccount::derive_pda(program_id, &user_wallet, account_index);
        if user_account_info.key != &user_account_pda {
            msg!("❌ Invalid UserAccount PDA");
            return Err(VaultError::InvalidPda.into());
        }

        // 4. 验证账户存在
        if user_account_info.data_is_empty() {
            msg!("❌ UserAccount does not exist for {}", user_wallet);
            return Err(VaultError::NotInitialized.into());
        }

        // 5. 扣除用户余额 (OC-M2 discriminator check)
        deserialize_checked(&user_account_info.data.borrow(), UserAccount::DISCRIMINATOR)?;
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
        
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
    /// 1. 验证 Governance Authority 签名
    /// 2. 扣除用户 Vault 余额
    /// 3. 从 Vault Token Account 转 USDC 到 Relayer Token Account
    ///
    /// Accounts:
    /// 0. `[signer]` Governance Authority/Relayer
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
        account_index: u8,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let relayer_token_account = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        // 1. Verify vault_authority (governance authority/relayer) is signed
        if !governance_authority.is_signer {
            msg!("RelayerWithdrawAndTransfer: vault_authority (governance authority) must sign");
            return Err(ProgramError::InvalidAccountData);
        }
        assert_writable(user_account_info)?;
        assert_writable(vault_token_account)?;
        assert_writable(relayer_token_account)?;

        // V-3: Verify VaultConfig PDA (must match other entrypoints, e.g. process_deposit / process_withdraw)
        let (expected_vault_config_pda, _) =
            Pubkey::find_program_address(&[b"vault_config"], program_id);
        if vault_config_info.key != &expected_vault_config_pda {
            msg!("❌ Invalid VaultConfig PDA");
            return Err(VaultError::InvalidAccount.into());
        }

        // OC-H2: Verify signer is governance_authority or authorized_caller
        let vault_config_data = vault_config_info.data.borrow();
        if !VaultConfig::is_valid_relayer_from_bytes(&vault_config_data, governance_authority.key) {
            msg!("❌ Invalid relayer: {} (not governance_authority nor authorized_caller)", governance_authority.key);
            return Err(VaultError::InvalidRelayer.into());
        }

        drop(vault_config_data);

        if amount == 0 {
            return Err(VaultError::InvalidAmount.into());
        }

        // 2. Verify UserAccount PDA derivation is correct (seeds match)
        let (user_account_pda, _bump) = UserAccount::derive_pda(program_id, &user_wallet, account_index);
        if user_account_info.key != &user_account_pda {
            msg!("RelayerWithdrawAndTransfer: UserAccount PDA derivation mismatch (expected {}, got {})", user_account_pda, user_account_info.key);
            return Err(ProgramError::InvalidAccountData);
        }

        if user_account_info.data_is_empty() {
            msg!("❌ UserAccount does not exist for {}", user_wallet);
            return Err(VaultError::NotInitialized.into());
        }

        deserialize_checked(&user_account_info.data.borrow(), UserAccount::DISCRIMINATOR)?;
        let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;

        if user_account.wallet != user_wallet {
            msg!("❌ Wallet mismatch: expected {}, got {}", user_wallet, user_account.wallet);
            return Err(VaultError::InvalidAccount.into());
        }

        // 3. Verify amount doesn't exceed user's available balance
        if user_account.available_balance_e6 < amount as i64 {
            msg!("RelayerWithdrawAndTransfer: amount {} exceeds available balance {}", amount, user_account.available_balance_e6);
            return Err(VaultError::InsufficientBalance.into());
        }

        // 4. Verify destination is not the vault itself (prevents self-referential transfer)
        if relayer_token_account.key == vault_token_account.key {
            msg!("RelayerWithdrawAndTransfer: destination cannot be the vault token account (self-referential transfer)");
            return Err(ProgramError::InvalidAccountData);
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
            amount, user_wallet, governance_authority.key, user_account.available_balance_e6);

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
        account_index: u8,
        token_index: u16,
    ) -> Result<u8, ProgramError> {
        let (expected_pda, bump) = derive_spot_token_balance_pda_with_index(program_id, wallet, account_index, token_index);
        if account_info.key != &expected_pda {
            msg!("❌ Invalid SpotTokenBalance PDA: expected={}, got={}, account_index={}", expected_pda, account_info.key, account_index);
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
        account_index: u8,
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
            &[account_index],
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

    /// Spot Token 入金 (用户直接调用)
    /// Accounts: user(signer) + balance_pda(w) + user_token + vault_token + vault_config + token_program + system_program
    fn process_spot_deposit(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
        account_index: u8,
        amount_e6: i64,
    ) -> ProgramResult {
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) must use Vault.Deposit, not SpotDeposit. Use Vault instruction #2.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        if amount == 0 || amount_e6 <= 0 {
            msg!("❌ Invalid amount: native={}, e6={}", amount, amount_e6);
            return Err(VaultError::InvalidAmount.into());
        }
        
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        // V-1: Verify token_program is a known SPL Token program
        if !token_compat::is_valid_token_program(token_program.key) {
            msg!("❌ Invalid token program: expected SPL Token or Token-2022");
            return Err(VaultError::InvalidAccount.into());
        }

        // S-1: Verify VaultConfig PDA
        let (expected_vault_config_pda, _) = Pubkey::find_program_address(&[b"vault_config"], program_id);
        if vault_config_info.key != &expected_vault_config_pda {
            msg!("❌ Invalid VaultConfig PDA");
            return Err(VaultError::InvalidPda.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.is_paused {
            return Err(VaultError::VaultPaused.into());
        }

        // S-2: Verify vault_token_account is owned by vault_config PDA (prevents
        // attackers from passing their own token account as the deposit destination).
        // SPL Token Account layout: [mint(32), owner(32), ...]. The owner at bytes
        // 32..64 must be the VaultConfig PDA so that only the vault program can
        // authorize withdrawals from it.
        let vault_ta_data = vault_token_account.try_borrow_data()?;
        if vault_ta_data.len() < 64 {
            msg!("❌ vault_token_account is not a valid SPL token account");
            return Err(VaultError::InvalidAccount.into());
        }
        let vault_ta_mint = Pubkey::try_from(&vault_ta_data[0..32])
            .map_err(|_| VaultError::InvalidAccount)?;
        let vault_ta_owner = Pubkey::try_from(&vault_ta_data[32..64])
            .map_err(|_| VaultError::InvalidAccount)?;
        if vault_ta_owner != expected_vault_config_pda {
            msg!("❌ vault_token_account owner ({}) != VaultConfig PDA ({})", vault_ta_owner, expected_vault_config_pda);
            return Err(VaultError::InvalidAccount.into());
        }
        drop(vault_ta_data);

        // S-4: Cross-check mints — user's token account and vault's token account
        // must hold the same token type.
        let user_ta_data = user_token_account.try_borrow_data()?;
        if user_ta_data.len() < 32 {
            msg!("❌ user_token_account is not a valid SPL token account");
            return Err(VaultError::InvalidAccount.into());
        }
        let user_ta_mint = Pubkey::try_from(&user_ta_data[0..32])
            .map_err(|_| VaultError::InvalidAccount)?;
        drop(user_ta_data);
        if user_ta_mint != vault_ta_mint {
            msg!("❌ Mint mismatch: user={}, vault={}", user_ta_mint, vault_ta_mint);
            return Err(VaultError::InvalidAccount.into());
        }

        let bump = Self::verify_spot_balance_pda(balance_pda_info, program_id, user.key, account_index, token_index)?;

        let mut balance = Self::auto_init_spot_balance(
            user, balance_pda_info, system_program, program_id, user.key, account_index, token_index, bump,
        )?;

        token_compat::transfer(
            token_program, user_token_account, vault_token_account, user, amount, None,
        )?;

        balance.available_e6 = balance.available_e6.checked_add(amount_e6).ok_or(VaultError::Overflow)?;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        msg!("✅ SpotDeposit: token_index={}, amount_native={}, amount_e6={}", token_index, amount, amount_e6);
        Ok(())
    }

    /// Spot Token 出金 (用户直接调用)
    /// Accounts: user(signer) + balance_pda(w) + user_token + vault_token + vault_config + token_program
    fn process_spot_withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        token_index: u16,
        amount: u64,
        account_index: u8,
        amount_e6: i64,
    ) -> ProgramResult {
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) must use Vault.Withdraw, not SpotWithdraw. Use Vault instruction #3.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        if amount == 0 || amount_e6 <= 0 {
            msg!("❌ Invalid amount: native={}, e6={}", amount, amount_e6);
            return Err(VaultError::InvalidAmount.into());
        }
        
        let account_info_iter = &mut accounts.iter();
        let user = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let user_token_account = next_account_info(account_info_iter)?;
        let vault_token_account = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        assert_signer(user)?;

        // V-1: Verify token_program is a known SPL Token program
        if !token_compat::is_valid_token_program(token_program.key) {
            msg!("❌ Invalid token program: expected SPL Token or Token-2022");
            return Err(VaultError::InvalidAccount.into());
        }

        let (vault_config_pda, vault_config_bump) =
            Pubkey::find_program_address(&[b"vault_config"], program_id);
        if vault_config_info.key != &vault_config_pda {
            msg!("❌ Invalid VaultConfig PDA");
            return Err(VaultError::InvalidPda.into());
        }

        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        if vault_config.is_paused {
            return Err(VaultError::VaultPaused.into());
        }

        Self::verify_spot_balance_pda(balance_pda_info, program_id, user.key, account_index, token_index)?;

        deserialize_checked(&balance_pda_info.data.borrow(), SpotTokenBalance::DISCRIMINATOR)?;
        let mut balance = deserialize_account::<SpotTokenBalance>(&balance_pda_info.data.borrow())?;
        if balance.available_e6 < amount_e6 {
            msg!("❌ Insufficient balance: available_e6={}, required_e6={}", balance.available_e6, amount_e6);
            return Err(VaultError::InsufficientBalance.into());
        }

        balance.available_e6 = checked_sub(balance.available_e6, amount_e6)?;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;

        // S-3: Verify vault_token_account is owned by vault_config PDA
        let vault_ta_data = vault_token_account.try_borrow_data()?;
        if vault_ta_data.len() < 64 {
            msg!("❌ vault_token_account is not a valid SPL token account");
            return Err(VaultError::InvalidAccount.into());
        }
        let vault_ta_mint = Pubkey::try_from(&vault_ta_data[0..32])
            .map_err(|_| VaultError::InvalidAccount)?;
        let vault_ta_owner = Pubkey::try_from(&vault_ta_data[32..64])
            .map_err(|_| VaultError::InvalidAccount)?;
        if vault_ta_owner != vault_config_pda {
            msg!("❌ vault_token_account owner mismatch");
            return Err(VaultError::InvalidAccount.into());
        }
        drop(vault_ta_data);

        // S-5: Cross-check mints for withdrawal
        let user_ta_data = user_token_account.try_borrow_data()?;
        if user_ta_data.len() < 32 {
            return Err(VaultError::InvalidAccount.into());
        }
        let user_ta_mint = Pubkey::try_from(&user_ta_data[0..32])
            .map_err(|_| VaultError::InvalidAccount)?;
        drop(user_ta_data);
        if user_ta_mint != vault_ta_mint {
            msg!("❌ Mint mismatch: user={}, vault={}", user_ta_mint, vault_ta_mint);
            return Err(VaultError::InvalidAccount.into());
        }

        token_compat::transfer(
            token_program, vault_token_account, user_token_account, vault_config_info, amount,
            Some(&[b"vault_config", &[vault_config_bump]]),
        )?;

        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;
        msg!("✅ SpotWithdraw: token_index={}, amount_native={}, amount_e6={}", token_index, amount, amount_e6);
        Ok(())
    }

    /// Relayer 代理 Spot 入金
    /// Accounts: governance_authority(signer) + balance_pda(w) + vault_config + system_program
    fn process_relayer_spot_deposit(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        token_index: u16,
        amount: u64,
        account_index: u8,
        amount_e6: i64,
    ) -> ProgramResult {
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) must use RelayerDeposit (#25), not RelayerSpotDeposit.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        if amount_e6 <= 0 {
            msg!("❌ Invalid amount_e6: {} (must be positive)", amount_e6);
            return Err(VaultError::InvalidAmount.into());
        }
        
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        // OC-H2: Accept governance_authority OR authorized_caller
        if vault_config.governance_authority != *governance_authority.key
            && !vault_config.is_authorized_caller(governance_authority.key)
        {
            return Err(VaultError::InvalidRelayer.into());
        }

        let bump = Self::verify_spot_balance_pda(balance_pda_info, program_id, &user_wallet, account_index, token_index)?;
        let mut balance = Self::auto_init_spot_balance(
            governance_authority, balance_pda_info, system_program, program_id, &user_wallet, account_index, token_index, bump,
        )?;

        balance.available_e6 = balance.available_e6.checked_add(amount_e6).ok_or(VaultError::Overflow)?;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        msg!("✅ RelayerSpotDeposit: user={}, token_index={}, amount_native={}, amount_e6={}", user_wallet, token_index, amount, amount_e6);
        Ok(())
    }

    /// Relayer 代理 Spot 出金 (with SPL token transfer)
    /// Accounts: governance_authority(signer) + balance_pda(w) + vault_config + vault_token_account(w) + user_token_account(w) + token_program
    /// If only 3 accounts are passed (legacy), PDA is debited without SPL transfer (backward-compat).
    fn process_relayer_spot_withdraw(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        token_index: u16,
        amount: u64,
        account_index: u8,
        amount_e6: i64,
    ) -> ProgramResult {
        if token_index == 0 {
            msg!("❌ USDC (token_index=0) must use RelayerWithdraw (#26), not RelayerSpotWithdraw.");
            return Err(VaultError::QuoteAssetMustUseVaultPath.into());
        }

        if amount_e6 <= 0 {
            msg!("❌ Invalid amount_e6: {} (must be positive)", amount_e6);
            return Err(VaultError::InvalidAmount.into());
        }

        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;
        let vault_config = deserialize_account::<VaultConfig>(&vault_config_info.data.borrow())?;
        // OC-H2: Accept governance_authority OR authorized_caller
        if vault_config.governance_authority != *governance_authority.key
            && !vault_config.is_authorized_caller(governance_authority.key)
        {
            return Err(VaultError::InvalidRelayer.into());
        }

        Self::verify_spot_balance_pda(balance_pda_info, program_id, &user_wallet, account_index, token_index)?;
        deserialize_checked(&balance_pda_info.data.borrow(), SpotTokenBalance::DISCRIMINATOR)?;
        let mut balance = deserialize_account::<SpotTokenBalance>(&balance_pda_info.data.borrow())?;
        if balance.available_e6 < amount_e6 {
            msg!("❌ Insufficient balance: available_e6={}, required_e6={}", balance.available_e6, amount_e6);
            return Err(VaultError::InsufficientBalance.into());
        }
        balance.available_e6 = checked_sub(balance.available_e6, amount_e6)?;
        balance.last_update_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        // SPL token transfer: if additional accounts are provided, transfer real tokens.
        let vault_token_account = next_account_info(account_info_iter);
        if let Ok(vault_ta) = vault_token_account {
            let user_token_account = next_account_info(account_info_iter)?;
            let token_program = next_account_info(account_info_iter)?;

            if !token_compat::is_valid_token_program(token_program.key) {
                msg!("❌ Invalid token program for RelayerSpotWithdraw transfer");
                return Err(VaultError::InvalidAccount.into());
            }

            let (vault_config_pda, vault_config_bump) =
                Pubkey::find_program_address(&[b"vault_config"], program_id);
            if vault_config_info.key != &vault_config_pda {
                return Err(VaultError::InvalidPda.into());
            }

            let vault_ta_data = vault_ta.try_borrow_data()?;
            if vault_ta_data.len() < 64 {
                msg!("❌ vault_token_account is not a valid SPL token account");
                return Err(VaultError::InvalidAccount.into());
            }
            let vault_ta_mint = Pubkey::try_from(&vault_ta_data[0..32])
                .map_err(|_| VaultError::InvalidAccount)?;
            let vault_ta_owner = Pubkey::try_from(&vault_ta_data[32..64])
                .map_err(|_| VaultError::InvalidAccount)?;
            if vault_ta_owner != vault_config_pda {
                msg!("❌ vault_token_account not owned by vault_config PDA");
                return Err(VaultError::InvalidAccount.into());
            }
            drop(vault_ta_data);

            let user_ta_data = user_token_account.try_borrow_data()?;
            if user_ta_data.len() < 32 {
                return Err(VaultError::InvalidAccount.into());
            }
            let user_ta_mint = Pubkey::try_from(&user_ta_data[0..32])
                .map_err(|_| VaultError::InvalidAccount)?;
            drop(user_ta_data);

            if user_ta_mint != vault_ta_mint {
                msg!("❌ Mint mismatch: user={}, vault={}", user_ta_mint, vault_ta_mint);
                return Err(VaultError::InvalidAccount.into());
            }

            token_compat::transfer(
                token_program, vault_ta, user_token_account, vault_config_info, amount,
                Some(&[b"vault_config", &[vault_config_bump]]),
            )?;

            msg!("✅ RelayerSpotWithdraw+Transfer: user={}, token_index={}, amount_native={}, amount_e6={}", user_wallet, token_index, amount, amount_e6);
        } else {
            msg!("✅ RelayerSpotWithdraw (PDA-only): user={}, token_index={}, amount_e6={}", user_wallet, token_index, amount_e6);
        }

        Ok(())
    }

    // =========================================================================
    // Mirror: 链上 PDA 状态镜像
    // =========================================================================

    /// UserAccount PDA mirror (set-to-value, not increment).
    /// Relayer-only.
    ///
    /// # Relayer Verification Design (H-3)
    ///
    /// Vault uses `VaultConfig.governance_authority` (single key) for relayer authorization,
    /// while Exchange uses an `authorized_relayers` list. This is intentional:
    /// - Current deployment has exactly one Relayer key per environment.
    /// - Vault governance_authority == Relayer in all three environments (local/staging/mainnet).
    /// - If multi-Relayer support is needed in the future, add an
    ///   `authorized_relayers: Vec<Pubkey>` field to VaultConfig and upgrade
    ///   the on-chain program. This is a low-risk future change.
    fn process_user_account(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        account_index: u8,
        available_balance_e6: i64,
        locked_margin_e6: i64,
        spot_locked_e6: i64,
        oracle_locked_e6: i64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let user_account_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;
        assert_writable(user_account_info)?;

        let vault_config_data = vault_config_info.data.borrow();
        if vault_config_data.len() < 40 {
            return Err(VaultError::InvalidAccount.into());
        }
        // OC-H2: Verify signer is governance_authority or authorized_caller
        if !VaultConfig::is_valid_relayer_from_bytes(&vault_config_data, governance_authority.key) {
            msg!("UserAccount: invalid relayer {} (not governance_authority nor authorized_caller)", governance_authority.key);
            return Err(VaultError::InvalidRelayer.into());
        }

        let (user_account_pda, bump) = UserAccount::derive_pda(program_id, &user_wallet, account_index);
        if user_account_info.key != &user_account_pda {
            return Err(VaultError::InvalidPda.into());
        }

        if user_account_info.data_is_empty() {
            let rent = Rent::get()?;
            let space = USER_ACCOUNT_SIZE;
            let lamports = rent.minimum_balance(space);

            invoke_signed(
                &system_instruction::create_account(
                    governance_authority.key,
                    user_account_info.key,
                    lamports,
                    space as u64,
                    program_id,
                ),
                &[governance_authority.clone(), user_account_info.clone(), system_program.clone()],
                &[&[b"user", user_wallet.as_ref(), &[account_index], &[bump]]],
            )?;

            let user_account = UserAccount {
                discriminator: UserAccount::DISCRIMINATOR,
                wallet: user_wallet,
                bump,
                available_balance_e6,
                locked_margin_e6,
                unrealized_pnl_e6: 0,
                total_deposited_e6: 0,
                total_withdrawn_e6: 0,
                last_update_ts: solana_program::clock::Clock::get()?.unix_timestamp,
                spot_locked_e6,
                account_index,
                oracle_locked_e6,
                reserved: [0; 47],
            };
            user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;
        } else {
            let mut user_account = deserialize_account::<UserAccount>(&user_account_info.data.borrow())?;
            if user_account.wallet != user_wallet {
                return Err(VaultError::InvalidAccount.into());
            }

            let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
            if current_ts < user_account.last_update_ts {
                msg!("UserAccount: stale update rejected (current={} < stored={})",
                    current_ts, user_account.last_update_ts);
                return Ok(());
            }

            user_account.available_balance_e6 = available_balance_e6;
            user_account.locked_margin_e6 = locked_margin_e6;
            user_account.spot_locked_e6 = spot_locked_e6;
            user_account.oracle_locked_e6 = oracle_locked_e6;
            user_account.last_update_ts = current_ts;
            user_account.serialize(&mut &mut user_account_info.data.borrow_mut()[..])?;
        }

        msg!("UserAccount: wallet={} idx={} avail={} locked={} spot={} oracle={}",
            user_wallet, account_index, available_balance_e6, locked_margin_e6, spot_locked_e6, oracle_locked_e6);
        Ok(())
    }

    /// SpotTokenBalance PDA mirror (set-to-value, not increment).
    /// Relayer-only.
    fn process_spot_token_balance(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        user_wallet: Pubkey,
        account_index: u8,
        token_index: u16,
        available_e6: i64,
        locked_e6: i64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let balance_pda_info = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let system_program = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;
        assert_writable(balance_pda_info)?;

        let vault_config_data = vault_config_info.data.borrow();
        if vault_config_data.len() < 40 {
            return Err(VaultError::InvalidAccount.into());
        }
        // OC-H2: Verify signer is governance_authority or authorized_caller
        if !VaultConfig::is_valid_relayer_from_bytes(&vault_config_data, governance_authority.key) {
            msg!("SpotTokenBalance: invalid relayer {} (not governance_authority nor authorized_caller)", governance_authority.key);
            return Err(VaultError::InvalidRelayer.into());
        }

        let (balance_pda, bump) = derive_spot_token_balance_pda_with_index(
            program_id, &user_wallet, account_index, token_index,
        );
        if balance_pda_info.key != &balance_pda {
            return Err(VaultError::InvalidPda.into());
        }

        let mut balance = Self::auto_init_spot_balance(
            governance_authority, balance_pda_info, system_program, program_id,
            &user_wallet, account_index, token_index, bump,
        )?;

        let current_ts = solana_program::clock::Clock::get()?.unix_timestamp;
        if current_ts < balance.last_update_ts {
            msg!("SpotTokenBalance: stale update rejected (current={} < stored={})",
                current_ts, balance.last_update_ts);
            return Ok(());
        }

        balance.available_e6 = available_e6;
        balance.locked_e6 = locked_e6;
        balance.last_update_ts = current_ts;
        balance.serialize(&mut &mut balance_pda_info.data.borrow_mut()[..])?;

        msg!("SpotTokenBalance: wallet={} idx={} token={} avail={} locked={}",
            user_wallet, account_index, token_index, available_e6, locked_e6);
        Ok(())
    }

    /// Migrate VaultConfig from V1 (569 bytes) to V2 (505 bytes).
    ///
    /// Removes the deprecated ledger_program (32 bytes) and fund_program (32 bytes)
    /// fields from the on-chain data, compacting the account.
    ///
    /// V1 layout (569 bytes):
    ///   disc(8) + governance_authority(32) + usdc_mint(32) + vault_token_account(32)
    ///   + authorized_callers(320) + ledger_program(32) + fund_program(32)
    ///   + delegation_program(32) + total_deposits(8) + total_locked(8)
    ///   + is_paused(1) + reserved(32)
    ///
    /// V2 layout (505 bytes):
    ///   disc(8) + governance_authority(32) + usdc_mint(32) + vault_token_account(32)
    ///   + authorized_callers(320) + delegation_program(32)
    ///   + total_deposits(8) + total_locked(8) + is_paused(1) + reserved(32)
    fn process_migrate_vault_config(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let governance_authority = next_account_info(account_info_iter)?;
        let vault_config_info = next_account_info(account_info_iter)?;
        let _system_program = next_account_info(account_info_iter)?;

        assert_signer(governance_authority)?;

        let (vault_config_pda, _) =
            Pubkey::find_program_address(&[b"vault_config"], program_id);
        if vault_config_info.key != &vault_config_pda {
            return Err(VaultError::InvalidPda.into());
        }

        let data = vault_config_info.data.borrow();
        let current_len = data.len();

        if current_len == VAULT_CONFIG_SIZE {
            msg!("VaultConfig already migrated to V2 (505 bytes)");
            return Err(ProgramError::InvalidAccountData);
        }

        if current_len != VAULT_CONFIG_SIZE_V1 {
            msg!("VaultConfig unexpected size: {} (expected V1={})", current_len, VAULT_CONFIG_SIZE_V1);
            return Err(ProgramError::InvalidAccountData);
        }

        // Verify governance_authority matches stored governance_authority (bytes 8..40)
        let stored_governance_authority = Pubkey::try_from(&data[8..40])
            .map_err(|_| ProgramError::InvalidAccountData)?;
        if &stored_governance_authority != governance_authority.key {
            msg!("MigrateVaultConfig: governance_authority mismatch");
            return Err(VaultError::UnauthorizedUser.into());
        }

        // Parse V1 data by byte offsets:
        // [0..424]   = disc + governance_authority + usdc_mint + vault_token_account + authorized_callers
        // [424..456] = ledger_program (SKIP)
        // [456..488] = fund_program (SKIP)
        // [488..569] = delegation_program(32) + total_deposits(8) + total_locked(8) + is_paused(1) + reserved(32)
        let prefix = data[0..424].to_vec();     // 424 bytes (before ledger_program)
        let suffix = data[488..569].to_vec();   // 81 bytes (delegation_program onward)
        drop(data);

        // Compact: prefix(424) + suffix(81) = 505 bytes
        let mut new_data = Vec::with_capacity(VAULT_CONFIG_SIZE);
        new_data.extend_from_slice(&prefix);
        new_data.extend_from_slice(&suffix);
        assert_eq!(new_data.len(), VAULT_CONFIG_SIZE);

        // Realloc the account to 505 bytes
        vault_config_info.realloc(VAULT_CONFIG_SIZE, false)?;

        // Write the compacted data
        vault_config_info.data.borrow_mut()[..VAULT_CONFIG_SIZE].copy_from_slice(&new_data);

        msg!("MigrateVaultConfig: success (569 → 505 bytes)");
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
