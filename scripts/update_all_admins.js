/**
 * Update admin for ALL remaining on-chain programs
 * 
 * Changes admin from faucet (EdwhYb4D...) to relayer1 (Ey5dD4Qz...)
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
const FAUCET_KEY_PATH = '/Users/chuciqin/Desktop/project1024/1024codebase/1024chain-config-production/keys/mainnet/node1/faucet.json';
const RELAYER1_KEY_PATH = '/Users/chuciqin/Desktop/project1024/1024codebase/1024chain-config-production/keys/mainnet/node1/relayers/relayer1.json';

// All deprecated programs (Exchange Ledger, Fund, Listing) have been removed.
// Only Vault and Exchange programs remain active.
// Vault admin is updated via its own UpdateAdmin instruction.
// Exchange admin is updated via its own UpdateAdmin instruction.
const PROGRAMS = [];

async function main() {
  const connection = new Connection(RPC_URL, 'confirmed');

  const faucetSecret = JSON.parse(fs.readFileSync(FAUCET_KEY_PATH, 'utf8'));
  const currentAdmin = Keypair.fromSecretKey(Uint8Array.from(faucetSecret));

  const relayer1Secret = JSON.parse(fs.readFileSync(RELAYER1_KEY_PATH, 'utf8'));
  const newAdmin = Keypair.fromSecretKey(Uint8Array.from(relayer1Secret));

  console.log('=== Batch UpdateAdmin for all programs ===');
  console.log(`Current Admin (faucet): ${currentAdmin.publicKey.toBase58()}`);
  console.log(`New Admin (relayer1):   ${newAdmin.publicKey.toBase58()}`);
  console.log('');

  for (const prog of PROGRAMS) {
    console.log(`--- ${prog.name} ---`);
    console.log(`  Program: ${prog.programId}`);
    console.log(`  Config:  ${prog.configPda}`);
    console.log(`  IX idx:  ${prog.updateAdminIx}`);

    const data = Buffer.alloc(1 + 32);
    data.writeUInt8(prog.updateAdminIx, 0);
    newAdmin.publicKey.toBuffer().copy(data, 1);

    const keys = [
      { pubkey: currentAdmin.publicKey, isSigner: true, isWritable: prog.extraAccounts },
      { pubkey: new PublicKey(prog.configPda), isSigner: false, isWritable: true },
    ];

    if (prog.extraAccounts) {
      keys.push({ pubkey: newAdmin.publicKey, isSigner: false, isWritable: false });
    }

    const ix = new TransactionInstruction({
      programId: new PublicKey(prog.programId),
      keys,
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

      console.log(`  ✅ Success! Signature: ${signature}`);
    } catch (error) {
      console.error(`  ❌ Failed:`);
      if (error.logs) {
        error.logs.forEach(log => console.error(`    ${log}`));
      }
      console.error(`    ${error.message || error}`);
    }
    console.log('');
  }

  // Now update RelayerConfig admin (needs InitializeRelayers with same relayers but new admin,
  // or a program upgrade - flag for manual attention)
  console.log('=== RelayerConfig & PM Fee Config ===');
  console.log('⚠️  RelayerConfig (FDsvVHYZ...): No UpdateAdmin instruction exists.');
  console.log('    Only needed for AddRelayer/RemoveRelayer operations.');
  console.log('    Requires program upgrade to fix.');
  console.log('');
  console.log('⚠️  PM Fee Config (D8rwT8xY...): Managed by Fund program.');
  console.log('    Will be accessible after Fund authority update above.');
  console.log('');

  console.log('=== Re-checking all admins ===');
}

main();
