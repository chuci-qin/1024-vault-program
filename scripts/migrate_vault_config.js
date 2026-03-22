/**
 * Migrate VaultConfig from V1 (569 bytes) to V2 (505 bytes)
 * 
 * Removes deprecated ledger_program and fund_program fields.
 * Must be run AFTER deploying the updated Vault Program.
 * 
 * Usage:
 *   node migrate_vault_config.js <environment>
 *   environment: local | stable | mainnet
 */

const {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
  SystemProgram,
  sendAndConfirmTransaction,
} = require('@solana/web3.js');
const fs = require('fs');

const BASE = '/Users/chuciqin/Desktop/project1024/1024codebase/1024chain-config-production/keys';

const ENVS = {
  local: {
    rpc: 'https://rpc-testnet.1024chain.com/rpc/',
    adminKey: `${BASE}/local-testnet/node1/relayers/relayer1.json`,
    vaultProgramId: 'EKsHPHtZmHRH9TFNGPVFp7MWNFuBcYZj1mdv87F9aSNt',
    vaultConfigPda: 'JAsjv6LQVpv1BJCBnt2Yf2nuf3ATdRncTaYUBm3TaKxR',
  },
  stable: {
    rpc: 'https://rpc-testnet-stable.1024chain.com',
    adminKey: `${BASE}/testnet-stable/node1/relayers/relayer1.json`,
    vaultProgramId: 'BxMAToJxZYZ2iTrFL4cRAVL9pHZyakMvjbk1LTLHi9Nh',
    vaultConfigPda: 'ETJUkA9tKjLWuZ3mk7EwVzB1y4HQcmPVRHheonuCAMf8',
  },
  mainnet: {
    rpc: 'https://rpc.1024chain.com',
    adminKey: `${BASE}/mainnet/node1/relayers/relayer1.json`,
    vaultProgramId: 'C3pDwbciRtrxDr2Qfuqw67EUb9DHBJsAnmhty1jfk9fF',
    vaultConfigPda: 'DMjPWD5GBzxfeSeqiscqN3hNiJqzsk5QBJqkXv9tQjeC',
  },
};

const MIGRATE_VAULT_CONFIG_INDEX = 54;

async function main() {
  const envName = process.argv[2];
  if (!envName || !ENVS[envName]) {
    console.error('Usage: node migrate_vault_config.js <local|stable|mainnet>');
    process.exit(1);
  }

  const env = ENVS[envName];
  console.log(`\n=== MigrateVaultConfig: ${envName} ===`);
  console.log(`RPC: ${env.rpc}`);
  console.log(`Vault Program: ${env.vaultProgramId}`);
  console.log(`VaultConfig PDA: ${env.vaultConfigPda}`);

  const connection = new Connection(env.rpc, 'confirmed');

  const adminSecret = JSON.parse(fs.readFileSync(env.adminKey, 'utf8'));
  const admin = Keypair.fromSecretKey(new Uint8Array(adminSecret));
  console.log(`Admin: ${admin.publicKey.toBase58()}`);

  const accountInfo = await connection.getAccountInfo(new PublicKey(env.vaultConfigPda));
  if (!accountInfo) {
    console.error('ERROR: VaultConfig PDA not found on chain!');
    process.exit(1);
  }
  console.log(`Current VaultConfig size: ${accountInfo.data.length} bytes`);

  if (accountInfo.data.length === 505) {
    console.log('VaultConfig already migrated to V2 (505 bytes). Nothing to do.');
    return;
  }
  if (accountInfo.data.length !== 569) {
    console.error(`ERROR: Unexpected VaultConfig size: ${accountInfo.data.length} (expected 569)`);
    process.exit(1);
  }

  console.log('VaultConfig is V1 (569 bytes). Proceeding with migration...');

  const instructionData = Buffer.from([MIGRATE_VAULT_CONFIG_INDEX]);

  const ix = new TransactionInstruction({
    programId: new PublicKey(env.vaultProgramId),
    keys: [
      { pubkey: admin.publicKey, isSigner: true, isWritable: false },
      { pubkey: new PublicKey(env.vaultConfigPda), isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data: instructionData,
  });

  const tx = new Transaction().add(ix);

  try {
    const sig = await sendAndConfirmTransaction(connection, tx, [admin], {
      commitment: 'confirmed',
    });
    console.log(`\nMigration TX signature: ${sig}`);
  } catch (err) {
    console.error('Migration TX failed:', err.message);
    if (err.logs) {
      console.error('Logs:', err.logs.join('\n'));
    }
    process.exit(1);
  }

  const updatedInfo = await connection.getAccountInfo(new PublicKey(env.vaultConfigPda));
  if (updatedInfo) {
    console.log(`\nPost-migration VaultConfig size: ${updatedInfo.data.length} bytes`);
    if (updatedInfo.data.length === 505) {
      console.log('SUCCESS: VaultConfig migrated to V2 (505 bytes)!');
    } else {
      console.error(`WARNING: Unexpected post-migration size: ${updatedInfo.data.length}`);
    }
  }
}

main().catch(console.error);
