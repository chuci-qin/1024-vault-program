//! Vault Program Utility Functions

use crate::error::VaultError;
use solana_program::{
    account_info::AccountInfo,
    program_error::ProgramError,
    pubkey::Pubkey,
};

/// 验证账户所有者
pub fn assert_owned_by(account: &AccountInfo, owner: &Pubkey) -> Result<(), ProgramError> {
    if account.owner != owner {
        Err(VaultError::InvalidAccount.into())
    } else {
        Ok(())
    }
}

/// 验证账户是否为signer
pub fn assert_signer(account: &AccountInfo) -> Result<(), ProgramError> {
    if !account.is_signer {
        Err(ProgramError::MissingRequiredSignature)
    } else {
        Ok(())
    }
}

/// 验证账户是否可写
pub fn assert_writable(account: &AccountInfo) -> Result<(), ProgramError> {
    if !account.is_writable {
        Err(ProgramError::InvalidAccountData)
    } else {
        Ok(())
    }
}

/// 安全的 i64 加法
pub fn checked_add(a: i64, b: i64) -> Result<i64, ProgramError> {
    a.checked_add(b).ok_or(VaultError::Overflow.into())
}

/// 安全的 i64 减法
pub fn checked_sub(a: i64, b: i64) -> Result<i64, ProgramError> {
    a.checked_sub(b).ok_or(VaultError::Overflow.into())
}

/// 安全的 u64 加法
pub fn checked_add_u64(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_add(b).ok_or(VaultError::Overflow.into())
}

/// 安全的 u64 减法
pub fn checked_sub_u64(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_sub(b).ok_or(VaultError::Overflow.into())
}

/// 获取当前时间戳
/// 
/// 注意: 在 BPF 环境中应使用 Clock sysvar
/// 这里提供简化实现用于开发
pub fn get_current_timestamp() -> i64 {
    #[cfg(target_os = "solana")]
    {
        use solana_program::clock::Clock;
        use solana_program::sysvar::Sysvar;
        Clock::get().map(|c| c.unix_timestamp).unwrap_or(0)
    }
    
    #[cfg(not(target_os = "solana"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checked_add() {
        assert_eq!(checked_add(100, 200).unwrap(), 300);
        assert_eq!(checked_add(-100, 200).unwrap(), 100);
        assert!(checked_add(i64::MAX, 1).is_err());
    }

    #[test]
    fn test_checked_sub() {
        assert_eq!(checked_sub(200, 100).unwrap(), 100);
        assert_eq!(checked_sub(100, 200).unwrap(), -100);
        assert!(checked_sub(i64::MIN, 1).is_err());
    }
}

