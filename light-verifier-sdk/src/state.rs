use crate::light_transaction::{Config, Transaction};
use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};
use std::marker::PhantomData;

/// Verifier state is a boiler plate struct which should be versatile enough to serve many use cases.
/// For specialized use cases with less
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct VerifierState10Ins<T: Config> {
    pub signer: Pubkey,
    pub nullifiers: Vec<Vec<u8>>,
    pub leaves: Vec<Vec<u8>>,
    pub public_amount: [u8; 32],
    pub fee_amount: [u8; 32],
    pub mint_pubkey: [u8; 32],
    pub merkle_root: [u8; 32],
    pub tx_integrity_hash: [u8; 32],
    pub relayer_fee: u64,
    pub encrypted_utxos: Vec<u8>,
    pub merkle_root_index: u64,
    pub checked_public_inputs: Vec<Vec<u8>>,
    pub e_phantom: PhantomData<T>,
}

impl<T: Config> VerifierState10Ins<T> {
    pub const LEN: usize = 2048;
}

impl<T: Config> anchor_lang::AccountDeserialize for VerifierState10Ins<T> {
    fn try_deserialize_unchecked(buf: &mut &[u8]) -> Result<Self> {
        match VerifierState10Ins::deserialize(buf) {
            Ok(v) => Ok(v),
            Err(_) => err!(anchor_lang::error::ErrorCode::AccountDidNotDeserialize),
        }
    }
}

impl<T: Config> anchor_lang::AccountSerialize for VerifierState10Ins<T> {
    fn try_serialize<W: std::io::Write>(&self, writer: &mut W) -> Result<()> {
        self.serialize(writer).unwrap();
        match self.serialize(writer) {
            Ok(_) => Ok(()),
            Err(_) => err!(anchor_lang::error::ErrorCode::AccountDidNotSerialize),
        }
    }
}

impl<T: Config> anchor_lang::Owner for VerifierState10Ins<T> {
    fn owner() -> Pubkey {
        Pubkey::new(&T::ID[..])
    }
}

impl<T: Config> From<Transaction<'_, '_, '_, T>> for VerifierState10Ins<T> {
    fn from(light_tx: Transaction<'_, '_, '_, T>) -> VerifierState10Ins<T> {
        assert_eq!(T::NR_LEAVES / 2, light_tx.leaves.len());
        assert_eq!(T::NR_NULLIFIERS, light_tx.nullifiers.len());
        // assert_eq!(T::NR_CHECKED_PUBLIC_INPUTS, light_tx.checked_public_inputs.len());

        // need to remove one nested layer because serde cannot handle three layered nesting
        let mut leaves = Vec::new();
        for pair in light_tx.leaves.iter() {
            leaves.push(pair[0].clone());
            leaves.push(pair[1].clone());
        }

        VerifierState10Ins {
            merkle_root_index: <usize as TryInto<u64>>::try_into(light_tx.merkle_root_index)
                .unwrap(),
            signer: Pubkey::new(&[0u8; 32]),
            nullifiers: light_tx.nullifiers,
            leaves,
            public_amount: light_tx.public_amount.try_into().unwrap(),
            fee_amount: light_tx.fee_amount.try_into().unwrap(),
            mint_pubkey: light_tx.mint_pubkey.try_into().unwrap(),
            relayer_fee: light_tx.relayer_fee,
            encrypted_utxos: [
                light_tx.encrypted_utxos.clone(),
                vec![0u8; 256 - light_tx.encrypted_utxos.len()],
            ]
            .concat(),
            merkle_root: light_tx.merkle_root.try_into().unwrap(),
            tx_integrity_hash: light_tx.tx_integrity_hash.try_into().unwrap(),
            checked_public_inputs: light_tx.checked_public_inputs,
            e_phantom: PhantomData,
        }
    }
}