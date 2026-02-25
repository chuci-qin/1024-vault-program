/**
 * Update Vault Program Admin
 * 
 * Changes the on-chain VaultConfig admin from faucet.json (EdwhYb4D...)
 * to relayer1.json (Ey5dD4Qz...) so that RelayerDeposit transactions succeed.
 * 
 * Usage: node update_vault_admin.js
 */

const {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
  sendAndConfirmTransaction,
} = require('@solana/web3.js');
const fs = require('fs');

const RPC_URL = 'https://rpc.1024chain.com';
const VAULT_PROGRAM_ID = new PublicKey('C3pDwbciRtrxDr2Qfuqw67EUb9DHBJsAnmhty1jfk9fF');
const VAULT_CONFIG_PDA = new PublicKey('DMjPWD5GBzxfeSeqiscqN3hNiJqzsk5QBJqkXv9tQjeC');

// Borsh enum index for UpdateAdmin in VaultInstruction
const UPDATE_ADMIN_IX = 11;

const FAUCET_KEY_PATH = '/Users/chuciqin/Desktop/project1024/1024codebase/1024chain-config-production/keys/mainnet/node1/faucet.json';
const RELAYER1_KEY_PATH = '/Users/chuciqin/Desktop/project1024/1024codebase/1024chain-config-production/keys/mainnet/node1/relayers/relayer1.json';

async function main() {
  const connection = new Connection(RPC_URL, 'confirmed');

  const faucetSecret = JSON.parse(fs.readFileSync(FAUCET_KEY_PATH, 'utf8'));
  const currentAdmin = Keypair.fromSecretKey(Uint8Array.from(faucetSecret));

  const relayer1Secret = JSON.parse(fs.readFileSync(RELAYER1_KEY_PATH, 'utf8'));
  const newAdmin = Keypair.fromSecretKey(Uint8Array.from(relayer1Secret));

  console.log('=== Vault UpdateAdmin ===');
  console.log(`Vault Program:  ${VAULT_PROGRAM_ID.toBase58()}`);
  console.log(`VaultConfig:    ${VAULT_CONFIG_PDA.toBase58()}`);
  console.log(`Current Admin:  ${currentAdmin.publicKey.toBase58()}`);
  console.log(`New Admin:      ${newAdmin.publicKey.toBase58()}`);
  console.log('');

  // Build instruction data: [u8 variant_index] + [32 bytes new_admin pubkey]
  const data = Buffer.alloc(1 + 32);
  data.writeUInt8(UPDATE_ADMIN_IX, 0);
  newAdmin.publicKey.toBuffer().copy(data, 1);

  const ix = new TransactionInstruction({
    programId: VAULT_PROGRAM_ID,
    keys: [
      { pubkey: currentAdmin.publicKey, isSigner: true, isWritable: false },
      { pubkey: VAULT_CONFIG_PDA, isSigner: false, isWritable: true },
    ],
    data,
  });

  const tx = new Transaction().add(ix);

  try {
    const { blockhash } = await connection.getLatestBlockhash();
    tx.recentBlockhash = blockhash;
    tx.feePayer = currentAdmin.publicKey;

    const signature = await sendAndConfirmTransaction(connection, tx, [currentAdmin], {
      commitment: 'confirmed',
    });

    console.log('✅ UpdateAdmin successful!');
    console.log(`Signature: ${signature}`);
    console.log(`Admin changed: ${currentAdmin.publicKey.toBase58()} → ${newAdmin.publicKey.toBase58()}`);
  } catch (error) {
    console.error('❌ UpdateAdmin failed:');
    if (error.logs) {
      error.logs.forEach(log => console.error('  ', log));
    }
    console.error(error.message || error);
    process.exit(1);
  }
}

main();
