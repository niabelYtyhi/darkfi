/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2023 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use darkfi_sdk::{
    crypto::{
        pasta_prelude::*, pedersen_commitment_base, pedersen_commitment_u64, Coin, ContractId,
        PublicKey, DARK_TOKEN_ID,
    },
    db::{db_contains_key, db_lookup},
    error::ContractError,
    msg,
    pasta::pallas,
    util::get_verifying_slot,
    ContractCall,
};
use darkfi_serial::{deserialize, serialize, Encodable, WriteExt};

use crate::{
    error::MoneyError,
    model::{MoneyMintParamsV1, MoneyMintUpdateV1},
    MoneyFunction, MONEY_CONTRACT_COINS_TREE, MONEY_CONTRACT_ZKAS_MINT_NS_V1,
};

/// `get_metadata` function for `Money::GenesisMintV1`
pub(crate) fn money_genesis_mint_get_metadata_v1(
    _cid: ContractId,
    call_idx: u32,
    calls: Vec<ContractCall>,
) -> Result<Vec<u8>, ContractError> {
    let self_ = &calls[call_idx as usize];
    let params: MoneyMintParamsV1 = deserialize(&self_.data[1..])?;

    // Public inputs for the ZK proofs we have to verify
    let mut zk_public_inputs: Vec<(String, Vec<pallas::Base>)> = vec![];
    // Public keys for the transaction signatures we have to verify
    let mut signature_pubkeys: Vec<PublicKey> = vec![];

    // Retrieve the pubkey from clear input
    signature_pubkeys.push(params.input.signature_public);

    // Grab the pedersen commitment from the anonymous output
    let value_coords = params.output.value_commit.to_affine().coordinates().unwrap();
    let token_coords = params.output.token_commit.to_affine().coordinates().unwrap();

    zk_public_inputs.push((
        MONEY_CONTRACT_ZKAS_MINT_NS_V1.to_string(),
        vec![
            params.output.coin.inner(),
            *value_coords.x(),
            *value_coords.y(),
            *token_coords.x(),
            *token_coords.y(),
        ],
    ));

    // Serialize everything gathered and return it
    let mut metadata = vec![];
    zk_public_inputs.encode(&mut metadata)?;
    signature_pubkeys.encode(&mut metadata)?;

    Ok(metadata)
}

/// `process_instruction` function for `Money::GenesisMintV1`
pub(crate) fn money_genesis_mint_process_instruction_v1(
    cid: ContractId,
    call_idx: u32,
    calls: Vec<ContractCall>,
) -> Result<Vec<u8>, ContractError> {
    let self_ = &calls[call_idx as usize];
    let params: MoneyMintParamsV1 = deserialize(&self_.data[1..])?;

    // Verify this contract call is verified against on genesis slot(0).
    let verifying_slot = get_verifying_slot();
    if verifying_slot != 0 {
        msg!("[GenesisMintV1] Error: Call is executed for slot {}, not genesis", verifying_slot);
        return Err(MoneyError::GenesisCallNonGenesisSlot.into())
    }

    // Only DARK_TOKEN_ID can be minted on genesis slot.
    if params.input.token_id != *DARK_TOKEN_ID {
        msg!("[GenesisMintV1] Error: Clear input used non-native token");
        return Err(MoneyError::TransferClearInputNonNativeToken.into())
    }

    // Access the necessary databases where there is information to
    // validate this state transition.
    let coins_db = db_lookup(cid, MONEY_CONTRACT_COINS_TREE)?;

    // Check that the coin from the output hasn't existed before.
    if db_contains_key(coins_db, &serialize(&params.output.coin))? {
        msg!("[GenesisMintV1] Error: Duplicate coin in output");
        return Err(MoneyError::DuplicateCoin.into())
    }

    // Verify that the value and token commitments match. In here we just
    // confirm that the clear input and the anon output have the same
    // commitments.
    if pedersen_commitment_u64(params.input.value, params.input.value_blind) !=
        params.output.value_commit
    {
        msg!("[GenesisMintV1] Error: Value commitment mismatch");
        return Err(MoneyError::ValueMismatch.into())
    }

    if pedersen_commitment_base(params.input.token_id.inner(), params.input.token_blind) !=
        params.output.token_commit
    {
        msg!("[GenesisMintV1] Error: Token commitment mismatch");
        return Err(MoneyError::TokenMismatch.into())
    }

    // Create a state update. We only need the new coin.
    let update = MoneyMintUpdateV1 { coin: Coin::from(params.output.coin) };
    let mut update_data = vec![];
    update_data.write_u8(MoneyFunction::MintV1 as u8)?;
    update.encode(&mut update_data)?;

    Ok(update_data)
}
