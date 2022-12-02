use anchor_lang::{
    prelude::*,
    solana_program::{msg, program_pack::Pack, sysvar},
};
use anchor_spl::token::Transfer;
use ark_ff::{
    bytes::{FromBytes, ToBytes},
    BigInteger, BigInteger256, Fp256, FpParameters, PrimeField,
};
use ark_std::{marker::PhantomData, vec::Vec};

use ark_bn254::{Fr, FrParameters};

use groth16_solana::groth16::{Groth16Verifier, Groth16Verifyingkey};

use crate::{
    accounts::Accounts,
    cpi_instructions::{
        insert_nullifiers_cpi, insert_two_leaves_cpi, withdraw_sol_cpi, withdraw_spl_cpi,
    },
    errors::VerifierSdkError,
    utils::{change_endianness, close_account::close_account},
};

use std::ops::Neg;

use merkle_tree_program::{
    program::MerkleTreeProgram,
    utils::{
        constants::{POOL_CONFIG_SEED, POOL_SEED},
        create_pda::create_and_check_pda,
    },
};

type G1 = ark_ec::short_weierstrass_jacobian::GroupAffine<ark_bn254::g1::Parameters>;

pub trait Config {
    /// Number of nullifiers to be inserted with the transaction.
    const NR_NULLIFIERS: usize;
    /// Number of output utxos.
    const NR_LEAVES: usize;
    /// Number of checked public inputs.
    const NR_CHECKED_PUBLIC_INPUTS: usize;
    /// Program ID of the verifier program.
    const ID: [u8; 32];
}

#[derive(Clone)]
pub struct Transaction<'info, 'a, 'c, T: Config> {
    pub merkle_root: Vec<u8>,
    pub public_amount: Vec<u8>,
    pub tx_integrity_hash: Vec<u8>,
    pub fee_amount: Vec<u8>,
    pub mint_pubkey: Vec<u8>,
    pub checked_public_inputs: Vec<Vec<u8>>,
    pub nullifiers: Vec<Vec<u8>>,
    pub leaves: Vec<Vec<Vec<u8>>>,
    pub relayer_fee: u64,
    pub proof_a: Vec<u8>,
    pub proof_b: Vec<u8>,
    pub proof_c: Vec<u8>,
    pub encrypted_utxos: Vec<u8>,
    pub pool_type: Vec<u8>,
    pub merkle_root_index: usize,
    pub transferred_funds: bool,
    pub computed_tx_integrity_hash: bool,
    pub verified_proof: bool,
    pub inserted_leaves: bool,
    pub inserted_nullifier: bool,
    pub fetched_root: bool,
    pub fetched_mint: bool,
    pub accounts: Option<&'a Accounts<'info, 'a, 'c>>,
    pub e_phantom: PhantomData<T>,
    pub verifyingkey: &'a Groth16Verifyingkey<'a>,
}

impl<T: Config> Transaction<'_, '_, '_, T> {
    #[allow(clippy::too_many_arguments)]
    pub fn new<'info, 'a, 'c>(
        proof: Vec<u8>,
        public_amount: Vec<u8>,
        fee_amount: Vec<u8>,
        checked_public_inputs: Vec<Vec<u8>>,
        nullifiers: Vec<Vec<u8>>,
        leaves: Vec<Vec<Vec<u8>>>,
        encrypted_utxos: Vec<u8>,
        relayer_fee: u64,
        merkle_root_index: usize,
        pool_type: Vec<u8>,
        accounts: Option<&'a Accounts<'info, 'a, 'c>>,
        verifyingkey: &'a Groth16Verifyingkey<'a>,
    ) -> Transaction<'info, 'a, 'c, T> {
        assert_eq!(T::NR_NULLIFIERS, nullifiers.len());
        assert_eq!(T::NR_LEAVES / 2, leaves.len());

        let proof_a: G1 =
            <G1 as FromBytes>::read(&*[&change_endianness(&proof[0..64])[..], &[0u8][..]].concat())
                .unwrap();
        let mut proof_a_neg = [0u8; 65];
        <G1 as ToBytes>::write(&proof_a.neg(), &mut proof_a_neg[..]).unwrap();
        Transaction {
            merkle_root: vec![0u8; 32],
            public_amount,
            tx_integrity_hash: vec![0u8; 32],
            fee_amount,
            mint_pubkey: vec![0u8; 32],
            checked_public_inputs,
            nullifiers,
            leaves,
            relayer_fee,
            proof_a: change_endianness(&proof_a_neg[..64]).to_vec(),
            proof_b: proof[64..64 + 128].to_vec(),
            proof_c: proof[64 + 128..256].to_vec(),
            encrypted_utxos,
            merkle_root_index,
            transferred_funds: false,
            computed_tx_integrity_hash: false,
            verified_proof: false,
            inserted_leaves: false,
            inserted_nullifier: false,
            fetched_root: false,
            fetched_mint: false,
            e_phantom: PhantomData,
            verifyingkey,
            accounts,
            pool_type,
        }
    }

    /// Transact is a wrapper function which computes the integrity hash, checks the root,
    /// verifies the zero knowledge proof, inserts leaves, inserts nullifiers, transfers funds and fees.
    pub fn transact(&mut self) -> Result<()> {
        self.compute_tx_integrity_hash()?;
        self.fetch_root()?;
        self.fetch_mint()?;
        self.verify()?;
        self.insert_leaves()?;
        self.insert_nullifiers()?;
        self.transfer_user_funds()?;
        self.transfer_fee()?;
        self.check_completion()
    }

    /// Verifies a Goth16 zero knowledge proof over the bn254 curve.
    pub fn verify(&mut self) -> Result<()> {
        if !self.computed_tx_integrity_hash {
            msg!("Tried to verify proof without computing integrity hash.");
        }

        if !self.fetched_mint {
            msg!("Tried to verify proof without fetching mind.");
        }

        if !self.fetched_root {
            msg!("Tried to verify proof without fetching root.");
        }

        let mut public_inputs = vec![
            self.merkle_root[..].to_vec(),
            self.public_amount[..].to_vec(),
            self.tx_integrity_hash[..].to_vec(),
            self.fee_amount[..].to_vec(),
            self.mint_pubkey[..].to_vec(),
        ];

        for input in self.nullifiers.iter() {
            public_inputs.push(input.to_vec());
        }

        for input in self.leaves.iter() {
            public_inputs.push(input[0].to_vec());
            public_inputs.push(input[1].to_vec());
        }

        for input in self.checked_public_inputs.iter() {
            public_inputs.push(input.to_vec());
        }

        let mut verifier = Groth16Verifier::new(
            self.proof_a.clone(),
            self.proof_b.clone(),
            self.proof_c.clone(),
            public_inputs,
            self.verifyingkey,
        )
        .unwrap();
        // self.verified_proof = true;
        // Ok(())
        match verifier.verify() {
            Ok(_) => {
                self.verified_proof = true;
                Ok(())
            }
            Err(e) => {
                msg!("Public Inputs:");
                msg!("merkle tree root {:?}", self.merkle_root);
                msg!("public_amount {:?}", self.public_amount);
                msg!("tx_integrity_hash {:?}", self.tx_integrity_hash);
                msg!("fee_amount {:?}", self.fee_amount);
                msg!("mint_pubkey {:?}", self.mint_pubkey);
                msg!("nullifiers {:?}", self.nullifiers);
                msg!("leaves {:?}", self.leaves);
                msg!("checked_public_inputs {:?}", self.checked_public_inputs);
                msg!("error {:?}", e);
                err!(VerifierSdkError::ProofVerificationFailed)
            }
        }
    }

    /// Computes the integrity hash of the transaction. This hash is an input to the ZKP, and
    /// ensures that the relayer cannot change parameters of the internal or unshield transaction.
    /// H(recipient||recipient_fee||signer||relayer_fee||encrypted_utxos).
    pub fn compute_tx_integrity_hash(&mut self) -> Result<()> {
        let input = [
            self.accounts
                .unwrap()
                .recipient
                .as_ref()
                .unwrap()
                .key()
                .to_bytes()
                .to_vec(),
            self.accounts
                .unwrap()
                .recipient_fee
                .as_ref()
                .unwrap()
                .key()
                .to_bytes()
                .to_vec(),
            self.accounts
                .unwrap()
                .signing_address
                .key()
                .to_bytes()
                .to_vec(),
            self.relayer_fee.to_le_bytes().to_vec(),
            self.encrypted_utxos.clone(),
        ]
        .concat();
        // msg!("recipient: {:?}", self.accounts.unwrap().recipient.as_ref().unwrap().key().to_bytes().to_vec());
        // msg!("recipient_fee: {:?}", self.accounts.unwrap().recipient_fee.as_ref().unwrap().key().to_bytes().to_vec());
        // msg!("signing_address: {:?}", self.accounts.unwrap().signing_address.key().to_bytes().to_vec());
        // msg!("relayer_fee: {:?}", self.relayer_fee.to_le_bytes().to_vec());
        // msg!("relayer_fee {}", self.relayer_fee);
        // msg!("integrity_hash inputs: {:?}", input);
        // msg!("integrity_hash inputs.len(): {}", input.len());
        let hash = Fr::from_be_bytes_mod_order(
            &anchor_lang::solana_program::keccak::hash(&input[..]).try_to_vec()?[..],
        );
        let mut bytes = Vec::<u8>::new();
        <Fp256<FrParameters> as ToBytes>::write(&hash, &mut bytes).unwrap();
        self.tx_integrity_hash = change_endianness(&bytes[..32]);
        // msg!("tx_integrity_hash be: {:?}", self.tx_integrity_hash);
        // msg!("Fq::from_be_bytes_mod_order(&hash[..]) : {}", hash);
        self.computed_tx_integrity_hash = true;
        Ok(())
    }

    /// Fetches the root according to an index from the passed-in Merkle tree.
    pub fn fetch_root(&mut self) -> Result<()> {
        let merkle_tree = self.accounts.unwrap().merkle_tree.load()?;
        self.merkle_root =
            change_endianness(merkle_tree.roots[self.merkle_root_index].to_vec().as_ref());
        self.fetched_root = true;
        Ok(())
    }

    /// Fetches the token mint from passed in sender account. If the sender account is not a
    /// token account, native mint is assumed.
    pub fn fetch_mint(&mut self) -> Result<()> {
        match spl_token::state::Account::unpack(
            &self
                .accounts
                .unwrap()
                .sender
                .as_ref()
                .unwrap()
                .data
                .borrow(),
        ) {
            Ok(sender_mint) => {
                // Omits the last byte for the mint pubkey bytes to fit into the bn254 field.
                self.mint_pubkey = [vec![0u8], sender_mint.mint.to_bytes()[..31].to_vec()].concat();
                self.fetched_mint = true;
                Ok(())
            }
            Err(_) => {
                self.mint_pubkey = vec![0u8; 32];
                self.fetched_mint = true;
                Ok(())
            }
        }
    }

    /// Calls merkle tree via cpi to insert leaves.
    pub fn insert_leaves(&mut self) -> Result<()> {
        if !self.verified_proof {
            msg!("Tried to insert leaves without verifying the proof.");
            return err!(VerifierSdkError::ProofNotVerified);
        }

        if T::NR_NULLIFIERS != self.nullifiers.len() {
            msg!(
                "NR_NULLIFIERS  {} != self.nullifiers.len() {}",
                T::NR_NULLIFIERS,
                self.nullifiers.len()
            );
            return err!(VerifierSdkError::InvalidNrNullifieraccounts);
        }

        if T::NR_NULLIFIERS + (T::NR_LEAVES / 2) != self.accounts.unwrap().remaining_accounts.len()
        {
            msg!(
                "NR_NULLIFIERS  {} != self.nullifiers.len() {}",
                T::NR_NULLIFIERS,
                self.nullifiers.len()
            );
            return err!(VerifierSdkError::InvalidNrLeavesaccounts);
        }

        // check merkle tree
        for (i, leaves) in self.leaves.iter().enumerate() {
            // check account integrities
            insert_two_leaves_cpi(
                self.accounts.unwrap().program_id,
                &self.accounts.unwrap().program_merkle_tree.to_account_info(),
                &self.accounts.unwrap().authority.to_account_info(),
                &self.accounts.unwrap().remaining_accounts[T::NR_NULLIFIERS + i].to_account_info(),
                &self
                    .accounts
                    .unwrap()
                    .pre_inserted_leaves_index
                    .to_account_info(),
                &self.accounts.unwrap().system_program.to_account_info(),
                &self
                    .accounts
                    .unwrap()
                    .registered_verifier_pda
                    .to_account_info(),
                change_endianness(&leaves[0]).try_into().unwrap(),
                change_endianness(&leaves[1]).try_into().unwrap(),
                self.accounts.unwrap().merkle_tree.key(),
                self.encrypted_utxos.clone(),
            )?;
        }

        self.inserted_leaves = true;
        Ok(())
    }

    /// Calls merkle tree via cpi to insert nullifiers.
    pub fn insert_nullifiers(&mut self) -> Result<()> {
        if !self.verified_proof {
            msg!("Tried to insert nullifiers without verifying the proof.");
            return err!(VerifierSdkError::ProofNotVerified);
        }

        if T::NR_NULLIFIERS != self.nullifiers.len() {
            msg!(
                "NR_NULLIFIERS  {} != self.nullifiers.len() {}",
                T::NR_NULLIFIERS,
                self.nullifiers.len()
            );
            return err!(VerifierSdkError::InvalidNrNullifieraccounts);
        }

        if T::NR_NULLIFIERS + (T::NR_LEAVES / 2) != self.accounts.unwrap().remaining_accounts.len()
        {
            msg!(
                "NR_LEAVES / 2  {} != self.leaves.len() {}",
                T::NR_LEAVES / 2,
                self.leaves.len()
            );
            return err!(VerifierSdkError::InvalidNrLeavesaccounts);
        }

        insert_nullifiers_cpi(
            self.accounts.unwrap().program_id,
            &self.accounts.unwrap().program_merkle_tree.to_account_info(),
            &self.accounts.unwrap().authority.to_account_info(),
            &self
                .accounts
                .unwrap()
                .system_program
                .to_account_info()
                .clone(),
            &self
                .accounts
                .unwrap()
                .registered_verifier_pda
                .to_account_info(),
            self.nullifiers.to_vec(),
            self.accounts.unwrap().remaining_accounts.to_vec(),
        )?;

        self.inserted_nullifier = true;
        Ok(())
    }

    /// Transfers user funds either to or from a merkle tree liquidity pool.
    pub fn transfer_user_funds(&mut self) -> Result<()> {
        if !self.verified_proof {
            msg!("Tried to transfer funds without verifying the proof.");
            return err!(VerifierSdkError::ProofNotVerified);
        }

        msg!("transferring user funds");
        // check mintPubkey
        let (pub_amount_checked, _) = self.check_amount(
            0,
            change_endianness(&self.public_amount).try_into().unwrap(),
        )?;

        // Only transfer if pub amount is greater than zero otherwise recipient and sender accounts are not checked
        if pub_amount_checked > 0 {
            let recipient_mint = spl_token::state::Account::unpack(
                &self
                    .accounts
                    .unwrap()
                    .recipient
                    .as_ref()
                    .unwrap()
                    .data
                    .borrow(),
            )?;
            let sender_mint = spl_token::state::Account::unpack(
                &self
                    .accounts
                    .unwrap()
                    .sender
                    .as_ref()
                    .unwrap()
                    .data
                    .borrow(),
            )?;

            // check mint
            if self.mint_pubkey[1..] != recipient_mint.mint.to_bytes()[..31] {
                msg!(
                    "*self.mint_pubkey[..31] {:?}, {:?}, recipient mint",
                    self.mint_pubkey[1..].to_vec(),
                    recipient_mint.mint.to_bytes()[..31].to_vec()
                );
                return err!(VerifierSdkError::InconsistentMintProofSenderOrRecipient);
            }
            if self.mint_pubkey[1..] != sender_mint.mint.to_bytes()[..31] {
                msg!(
                    "*self.mint_pubkey[..31] {:?}, {:?}, sender mint",
                    self.mint_pubkey[1..].to_vec(),
                    sender_mint.mint.to_bytes()[..31].to_vec()
                );
                return err!(VerifierSdkError::InconsistentMintProofSenderOrRecipient);
            }

            // is a token deposit or withdrawal
            if self.is_deposit() {
                self.check_spl_pool_account_derivation(
                    &self.accounts.unwrap().recipient.as_ref().unwrap().key(),
                    &recipient_mint.mint,
                )?;

                let seed = merkle_tree_program::ID.to_bytes();
                let (_, bump) = anchor_lang::prelude::Pubkey::find_program_address(
                    &[seed.as_ref()],
                    self.accounts.unwrap().program_id,
                );
                let bump = &[bump];
                let seeds = &[&[seed.as_slice(), bump][..]];

                let accounts = Transfer {
                    from: self
                        .accounts
                        .unwrap()
                        .sender
                        .as_ref()
                        .unwrap()
                        .to_account_info()
                        .clone(),
                    to: self
                        .accounts
                        .unwrap()
                        .recipient
                        .as_ref()
                        .unwrap()
                        .to_account_info()
                        .clone(),
                    authority: self.accounts.unwrap().authority.to_account_info().clone(),
                };

                let cpi_ctx = CpiContext::new_with_signer(
                    self.accounts
                        .unwrap()
                        .token_program
                        .unwrap()
                        .to_account_info()
                        .clone(),
                    accounts,
                    seeds,
                );
                anchor_spl::token::transfer(cpi_ctx, pub_amount_checked)?;
            } else {
                self.check_spl_pool_account_derivation(
                    &self.accounts.unwrap().sender.as_ref().unwrap().key(),
                    &sender_mint.mint,
                )?;

                // withdraw_spl_cpi
                withdraw_spl_cpi(
                    self.accounts.unwrap().program_id,
                    &self.accounts.unwrap().program_merkle_tree.to_account_info(),
                    &self.accounts.unwrap().authority.to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .sender
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .recipient
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .token_authority
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .token_program
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .registered_verifier_pda
                        .to_account_info(),
                    pub_amount_checked,
                )?;
            }
            msg!("transferred");
        }
        self.transferred_funds = true;
        Ok(())
    }

    /// Transfers the relayer fee  to or from a merkle tree liquidity pool.
    pub fn transfer_fee(&self) -> Result<()> {
        if !self.verified_proof {
            msg!("Tried to transfer fees without verifying the proof.");
            return err!(VerifierSdkError::ProofNotVerified);
        }

        // check that it is the native token pool
        let (fee_amount_checked, relayer_fee) = self.check_amount(
            self.relayer_fee,
            change_endianness(&self.fee_amount).try_into().unwrap(),
        )?;

        if fee_amount_checked > 0 {
            if self.is_deposit() {
                self.deposit_sol(
                    fee_amount_checked,
                    &self
                        .accounts
                        .unwrap()
                        .recipient_fee
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                )?;
            } else {
                self.check_sol_pool_account_derivation(
                    &self.accounts.unwrap().sender_fee.as_ref().unwrap().key(),
                )?;

                // withdraws sol for the user
                withdraw_sol_cpi(
                    self.accounts.unwrap().program_id,
                    &self.accounts.unwrap().program_merkle_tree.to_account_info(),
                    &self.accounts.unwrap().authority.to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .sender_fee
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .recipient_fee
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .registered_verifier_pda
                        .to_account_info(),
                    fee_amount_checked,
                )?;

                // pays the relayer fee
                withdraw_sol_cpi(
                    self.accounts.unwrap().program_id,
                    &self.accounts.unwrap().program_merkle_tree.to_account_info(),
                    &self.accounts.unwrap().authority.to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .sender_fee
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .relayer_recipient
                        .as_ref()
                        .unwrap()
                        .to_account_info(),
                    &self
                        .accounts
                        .unwrap()
                        .registered_verifier_pda
                        .to_account_info(),
                    relayer_fee,
                )?;
            }
        }

        Ok(())
    }

    /// Creates and closes an account such that deposited sol is part of the transaction fees.
    fn deposit_sol(&self, amount_checked: u64, recipient: &AccountInfo) -> Result<()> {
        self.check_sol_pool_account_derivation(&recipient.key())?;

        msg!("is deposit");
        let rent = <Rent as sysvar::Sysvar>::get()?;

        create_and_check_pda(
            self.accounts.unwrap().program_id,
            &self.accounts.unwrap().signing_address.to_account_info(),
            &self
                .accounts
                .unwrap()
                .escrow
                .as_ref()
                .unwrap()
                .to_account_info(),
            &self.accounts.unwrap().system_program.to_account_info(),
            &rent,
            &b"escrow"[..],
            &Vec::new(),
            0,              //bytes
            amount_checked, //lamports
            false,          //rent_exempt
        )?;
        close_account(
            &self
                .accounts
                .unwrap()
                .escrow
                .as_ref()
                .unwrap()
                .to_account_info(),
            recipient,
        )
    }

    /// Checks whether a transaction is a deposit by inspecting the public amount.
    pub fn is_deposit(&self) -> bool {
        if self.public_amount[24..] != [0u8; 8] && self.public_amount[..24] == [0u8; 24] {
            return true;
        }
        false
    }

    pub fn check_sol_pool_account_derivation(&self, pubkey: &Pubkey) -> Result<()> {
        let derived_pubkey = Pubkey::find_program_address(
            &[&[0u8; 32], &self.pool_type, POOL_CONFIG_SEED],
            &MerkleTreeProgram::id(),
        );

        if derived_pubkey.0 != *pubkey {
            return err!(VerifierSdkError::InvalidSenderorRecipient);
        }
        Ok(())
    }

    pub fn check_spl_pool_account_derivation(&self, pubkey: &Pubkey, mint: &Pubkey) -> Result<()> {
        let derived_pubkey = Pubkey::find_program_address(
            &[&mint.to_bytes(), &self.pool_type, POOL_SEED],
            &MerkleTreeProgram::id(),
        );

        if derived_pubkey.0 != *pubkey {
            return err!(VerifierSdkError::InvalidSenderorRecipient);
        }
        Ok(())
    }

    pub fn check_completion(&self) -> Result<()> {
        if self.transferred_funds
            && self.verified_proof
            && self.inserted_leaves
            && self.inserted_nullifier
        {
            return Ok(());
        }
        msg!("verified_proof {}", self.verified_proof);
        msg!("inserted_leaves {}", self.inserted_leaves);
        msg!("transferred_funds {}", self.transferred_funds);
        err!(VerifierSdkError::TransactionIncomplete)
    }

    #[allow(clippy::comparison_chain)]
    pub fn check_amount(&self, relayer_fee: u64, amount: [u8; 32]) -> Result<(u64, u64)> {
        // pub_amount is the public amount included in public inputs for proof verification
        let pub_amount = <BigInteger256 as FromBytes>::read(&amount[..]).unwrap();
        // Big integers are stored in 4 u64 limbs, if the number is <= U64::max() and encoded in little endian,
        // only the first limb is greater than 0.
        // Amounts in shielded accounts are limited to 64bit therefore a withdrawal will always be greater
        // than one U64::max().
        if pub_amount.0[0] > 0
            && pub_amount.0[1] == 0
            && pub_amount.0[2] == 0
            && pub_amount.0[3] == 0
        {
            if relayer_fee != 0 {
                msg!("relayer_fee {}", relayer_fee);
                return Err(VerifierSdkError::WrongPubAmount.into());
            }
            Ok((pub_amount.0[0], 0))
        } else if pub_amount.0[0] != 0 {
            // calculate ext_amount from pubAmount:
            let mut field = FrParameters::MODULUS;
            field.sub_noborrow(&pub_amount);

            // field.0[0] is the positive value
            if field.0[1] != 0 || field.0[2] != 0 || field.0[3] != 0 {
                msg!("Public amount is larger than u64.");
                return Err(VerifierSdkError::WrongPubAmount.into());
            }

            if field.0[0] < relayer_fee {
                msg!(
                    "Withdrawal invalid relayer_fee: pub amount {} < {} fee",
                    field.0[0],
                    relayer_fee
                );
                return Err(VerifierSdkError::WrongPubAmount.into());
            }

            Ok((field.0[0].saturating_sub(relayer_fee), relayer_fee))
        } else if pub_amount.0[0] == 0
            && pub_amount.0[1] == 0
            && pub_amount.0[2] == 0
            && pub_amount.0[3] == 0
        {
            Ok((0, 0))
        } else {
            Err(VerifierSdkError::WrongPubAmount.into())
        }
    }
}