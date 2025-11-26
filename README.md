# 1024 Vault Program

> 完全去中心化的资金托管程序

## 概述

1024 Vault Program 是 1024 DEX 的核心资金托管程序，负责：

- ✅ 用户资金托管 (100% 链上托管，平台不托管)
- ✅ 入金/出金操作
- ✅ 保证金锁定/释放
- ✅ 保险基金管理
- ✅ 清算结算
- ✅ CPI 权限验证

## 功能特性

### 用户操作

| 指令 | 功能 | 说明 |
|------|------|------|
| `InitializeUser` | 初始化用户账户 | 创建 UserAccount PDA |
| `Deposit` | 入金 | USDC 转入 Vault |
| `Withdraw` | 出金 | USDC 转出到用户钱包 |

### CPI 操作 (仅授权程序可调用)

| 指令 | 调用方 | 功能 |
|------|--------|------|
| `LockMargin` | Ledger | 开仓时锁定保证金 |
| `ReleaseMargin` | Ledger | 平仓时释放保证金 |
| `ClosePositionSettle` | Ledger | 平仓结算 (合并操作) |
| `LiquidatePosition` | Ledger | 清算 (原子操作) |

### 管理员操作

| 指令 | 功能 |
|------|------|
| `Initialize` | 初始化 Vault |
| `AddAuthorizedCaller` | 添加授权 Program |
| `RemoveAuthorizedCaller` | 移除授权 Program |
| `SetPaused` | 暂停/恢复 |
| `UpdateAdmin` | 更新管理员 |

## 账户结构

### VaultConfig (全局配置)

```rust
pub struct VaultConfig {
    pub admin: Pubkey,
    pub usdc_mint: Pubkey,
    pub vault_token_account: Pubkey,
    pub insurance_fund: Pubkey,
    pub authorized_callers: Vec<Pubkey>,  // CPI 白名单
    pub ledger_program: Pubkey,
    pub fund_program: Option<Pubkey>,
    pub delegation_program: Pubkey,
    pub total_deposits: u64,
    pub total_locked: u64,
    pub is_paused: bool,
}
```

### UserAccount (用户账户 PDA)

Seeds: `["user", wallet]`

```rust
pub struct UserAccount {
    pub wallet: Pubkey,
    pub available_balance_e6: i64,  // 可用余额
    pub locked_margin_e6: i64,       // 锁定保证金
    pub unrealized_pnl_e6: i64,      // 未实现盈亏
    pub total_deposited_e6: i64,     // 累计存款
    pub total_withdrawn_e6: i64,     // 累计提款
    // ...
}
```

### InsuranceFund (保险基金)

```rust
pub struct InsuranceFund {
    pub balance_e6: i64,
    pub total_liquidation_income_e6: i64,    // 清算收入
    pub total_fee_income_e6: i64,             // 手续费分成
    pub total_shortfall_payout_e6: i64,      // 穿仓支出
    pub adl_trigger_threshold_e6: i64,        // ADL 阈值
    pub adl_trigger_count: u64,               // ADL 次数
}
```

## 构建

```bash
# 编译检查
cargo check

# 运行测试
cargo test

# 构建 BPF 程序
cargo build-sbf
```

## 测试

当前测试覆盖：
- ✅ UserAccount equity 计算
- ✅ InsuranceFund cover_shortfall
- ✅ InsuranceFund should_trigger_adl
- ✅ Utils 安全数学运算

```bash
cargo test --lib
```

## 部署

```bash
# 部署到 Devnet
solana program deploy target/deploy/vault_program.so \
  --url devnet \
  --keypair ~/.config/solana/id.json

# 部署到 1024Chain Testnet
solana program deploy target/deploy/vault_program.so \
  --url https://testnet-rpc.1024chain.com/rpc/ \
  --keypair ~/.config/solana/id.json
```

## 安全特性

### CPI 权限验证

所有 CPI 指令（`LockMargin`, `ReleaseMargin`, `ClosePositionSettle`, `LiquidatePosition`）都会验证调用方是否在白名单中：

```rust
if !vault_config.is_authorized_caller(caller_program.key) {
    return Err(VaultError::UnauthorizedCaller);
}
```

### 保险基金机制

- 手续费分成 (10%) 流入保险基金
- 清算罚金归保险基金
- 穿仓损失由保险基金覆盖
- 保险基金不足时触发 ADL

## License

MIT


