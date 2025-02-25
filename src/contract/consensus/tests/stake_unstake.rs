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

//! Integration test of consensus staking and unstaking for Alice.
//!
//! We first airdrop Alica native tokes, and then she can stake and unstake
//! them a couple of times.
//!
//! With this test, we want to confirm the consensus contract state
//! transitions work for a single party and are able to be verified.
//!
//! TODO: Malicious cases

use std::time::{Duration, Instant};

use darkfi::{tx::Transaction, Result};
use darkfi_sdk::{
    crypto::{
        merkle_prelude::*, poseidon_hash, Coin, MerkleNode, Nullifier, CONSENSUS_CONTRACT_ID,
        MONEY_CONTRACT_ID,
    },
    ContractCall,
};
use darkfi_serial::{serialize, Encodable};
use log::info;
use rand::rngs::OsRng;

use darkfi_money_contract::{
    client::{
        stake_v1::MoneyStakeCallBuilder, unstake_v1::MoneyUnstakeCallBuilder, MoneyNote, OwnCoin,
    },
    MoneyFunction, CONSENSUS_CONTRACT_ZKAS_PROPOSAL_MINT_NS_V1,
    CONSENSUS_CONTRACT_ZKAS_PROPOSAL_REWARD_NS_V1, MONEY_CONTRACT_ZKAS_BURN_NS_V1,
    MONEY_CONTRACT_ZKAS_MINT_NS_V1,
};

use darkfi_consensus_contract::{
    client::{
        proposal_v1::ConsensusProposalCallBuilder, stake_v1::ConsensusStakeCallBuilder,
        unstake_v1::ConsensusUnstakeCallBuilder,
    },
    model::REWARD,
    ConsensusFunction,
};

mod harness;
use harness::{init_logger, ConsensusTestHarness};

#[async_std::test]
async fn consensus_contract_stake_unstake() -> Result<()> {
    init_logger();

    // Some benchmark averages
    let mut stake_sizes = vec![];
    let mut stake_broadcasted_sizes = vec![];
    let mut stake_creation_times = vec![];
    let mut stake_verify_times = vec![];
    let mut proposal_sizes = vec![];
    let mut proposal_broadcasted_sizes = vec![];
    let mut proposal_creation_times = vec![];
    let mut proposal_verify_times = vec![];
    let mut unstake_sizes = vec![];
    let mut unstake_broadcasted_sizes = vec![];
    let mut unstake_creation_times = vec![];
    let mut unstake_verify_times = vec![];

    // Some numbers we want to assert
    const ALICE_AIRDROP: u64 = 1000;

    // Slot to verify against
    let current_slot = 0;

    // Initialize harness
    let mut th = ConsensusTestHarness::new().await?;
    info!(target: "consensus", "[Faucet] ===================================================");
    info!(target: "consensus", "[Faucet] Building Money::Transfer params for Alice's airdrop");
    info!(target: "consensus", "[Faucet] ===================================================");
    let (airdrop_tx, airdrop_params) = th.airdrop_native(ALICE_AIRDROP, th.alice.keypair.public)?;
    let (money_mint_pk, money_mint_zkbin) =
        th.proving_keys.get(&MONEY_CONTRACT_ZKAS_MINT_NS_V1).unwrap();
    let (money_burn_pk, money_burn_zkbin) =
        th.proving_keys.get(&MONEY_CONTRACT_ZKAS_BURN_NS_V1).unwrap();
    let (proposal_reward_pk, proposal_reward_zkbin) =
        th.proving_keys.get(&CONSENSUS_CONTRACT_ZKAS_PROPOSAL_REWARD_NS_V1).unwrap();
    let (proposal_mint_pk, proposal_mint_zkbin) =
        th.proving_keys.get(&CONSENSUS_CONTRACT_ZKAS_PROPOSAL_MINT_NS_V1).unwrap();

    info!(target: "consensus", "[Faucet] ==========================");
    info!(target: "consensus", "[Faucet] Executing Alice airdrop tx");
    info!(target: "consensus", "[Faucet] ==========================");
    let erroneous_txs = th
        .faucet
        .state
        .read()
        .await
        .verify_transactions(&[airdrop_tx.clone()], current_slot, true)
        .await?;
    assert!(erroneous_txs.is_empty());
    th.faucet.merkle_tree.append(&MerkleNode::from(airdrop_params.outputs[0].coin.inner()));
    info!(target: "consensus", "[Alice] ==========================");
    info!(target: "consensus", "[Alice] Executing Alice airdrop tx");
    info!(target: "consensus", "[Alice] ==========================");
    let erroneous_txs = th
        .alice
        .state
        .read()
        .await
        .verify_transactions(&[airdrop_tx.clone()], current_slot, true)
        .await?;
    assert!(erroneous_txs.is_empty());
    th.alice.merkle_tree.append(&MerkleNode::from(airdrop_params.outputs[0].coin.inner()));

    assert!(th.faucet.merkle_tree.root(0).unwrap() == th.alice.merkle_tree.root(0).unwrap());

    // Gather new owncoin
    let leaf_position = th.alice.merkle_tree.witness().unwrap();
    let note: MoneyNote = airdrop_params.outputs[0].note.decrypt(&th.alice.keypair.secret)?;
    let alice_oc = OwnCoin {
        coin: Coin::from(airdrop_params.outputs[0].coin),
        note: note.clone(),
        secret: th.alice.keypair.secret,
        nullifier: Nullifier::from(poseidon_hash([th.alice.keypair.secret.inner(), note.serial])),
        leaf_position,
    };

    // Now Alice can stake her owncoin
    info!(target: "consensus", "[Alice] ============================");
    info!(target: "consensus", "[Alice] Building Money::Stake params");
    info!(target: "consensus", "[Alice] ============================");
    let timer = Instant::now();
    let alice_money_stake_call_debris = MoneyStakeCallBuilder {
        coin: alice_oc.clone(),
        tree: th.alice.merkle_tree.clone(),
        burn_zkbin: money_burn_zkbin.clone(),
        burn_pk: money_burn_pk.clone(),
    }
    .build()?;
    let (
        alice_money_stake_params,
        alice_money_stake_proofs,
        alice_money_stake_secret_key,
        alice_money_stake_value_blind,
    ) = (
        alice_money_stake_call_debris.params,
        alice_money_stake_call_debris.proofs,
        alice_money_stake_call_debris.signature_secret,
        alice_money_stake_call_debris.value_blind,
    );

    info!(target: "consensus", "[Alice] ================================");
    info!(target: "consensus", "[Alice] Building Consensus::Stake params");
    info!(target: "consensus", "[Alice] ================================");
    let alice_consensus_stake_call_debris = ConsensusStakeCallBuilder {
        coin: alice_oc.clone(),
        recipient: th.alice.keypair.public,
        value_blind: alice_money_stake_value_blind,
        token_blind: alice_money_stake_params.token_blind,
        nullifier: alice_money_stake_params.input.nullifier,
        merkle_root: alice_money_stake_params.input.merkle_root,
        signature_public: alice_money_stake_params.input.signature_public,
        mint_zkbin: money_mint_zkbin.clone(),
        mint_pk: money_mint_pk.clone(),
    }
    .build()?;
    let (alice_consensus_stake_params, alice_consensus_stake_proofs) =
        (alice_consensus_stake_call_debris.params, alice_consensus_stake_call_debris.proofs);

    info!(target: "consensus", "[Alice] =================");
    info!(target: "consensus", "[Alice] Building stake tx");
    info!(target: "consensus", "[Alice] =================");
    let mut data = vec![MoneyFunction::StakeV1 as u8];
    alice_money_stake_params.encode(&mut data)?;
    let money_call = ContractCall { contract_id: *MONEY_CONTRACT_ID, data };

    let mut data = vec![ConsensusFunction::StakeV1 as u8];
    alice_consensus_stake_params.encode(&mut data)?;
    let consensus_call = ContractCall { contract_id: *CONSENSUS_CONTRACT_ID, data };

    let calls = vec![money_call, consensus_call];
    let proofs = vec![alice_money_stake_proofs, alice_consensus_stake_proofs];
    let mut alice_stake_tx = Transaction { calls, proofs, signatures: vec![] };
    let money_sigs = alice_stake_tx.create_sigs(&mut OsRng, &[alice_money_stake_secret_key])?;
    let consensus_sigs = alice_stake_tx.create_sigs(&mut OsRng, &[alice_money_stake_secret_key])?;
    alice_stake_tx.signatures = vec![money_sigs, consensus_sigs];
    stake_creation_times.push(timer.elapsed());

    // Calculate transaction sizes
    let encoded: Vec<u8> = serialize(&alice_stake_tx);
    let size = ::std::mem::size_of_val(&*encoded);
    stake_sizes.push(size);
    let base58 = bs58::encode(&encoded).into_string();
    let size = ::std::mem::size_of_val(&*base58);
    stake_broadcasted_sizes.push(size);

    info!(target: "consensus", "[Faucet] ========================");
    info!(target: "consensus", "[Faucet] Executing Alice stake tx");
    info!(target: "consensus", "[Faucet] ========================");
    let timer = Instant::now();
    let erroneous_txs = th
        .faucet
        .state
        .read()
        .await
        .verify_transactions(&[alice_stake_tx.clone()], current_slot, true)
        .await?;
    assert!(erroneous_txs.is_empty());
    th.faucet
        .consensus_merkle_tree
        .append(&MerkleNode::from(alice_consensus_stake_params.output.coin.inner()));
    stake_verify_times.push(timer.elapsed());

    info!(target: "consensus", "[Alice] ========================");
    info!(target: "consensus", "[Alice] Executing Alice stake tx");
    info!(target: "consensus", "[Alice] ========================");
    let timer = Instant::now();
    let erroneous_txs = th
        .alice
        .state
        .read()
        .await
        .verify_transactions(&[alice_stake_tx.clone()], current_slot, true)
        .await?;
    assert!(erroneous_txs.is_empty());
    th.alice
        .consensus_merkle_tree
        .append(&MerkleNode::from(alice_consensus_stake_params.output.coin.inner()));
    stake_verify_times.push(timer.elapsed());

    assert!(th.faucet.merkle_tree.root(0).unwrap() == th.alice.merkle_tree.root(0).unwrap());
    assert!(
        th.faucet.consensus_merkle_tree.root(0).unwrap() ==
            th.alice.consensus_merkle_tree.root(0).unwrap()
    );

    // Gather new staked owncoin
    let leaf_position = th.alice.consensus_merkle_tree.witness().unwrap();
    let note: MoneyNote =
        alice_consensus_stake_params.output.note.decrypt(&th.alice.keypair.secret)?;
    let alice_staked_oc = OwnCoin {
        coin: Coin::from(alice_consensus_stake_params.output.coin),
        note: note.clone(),
        secret: th.alice.keypair.secret,
        nullifier: Nullifier::from(poseidon_hash([th.alice.keypair.secret.inner(), note.serial])),
        leaf_position,
    };

    // Verify values match
    assert!(alice_oc.note.value == alice_staked_oc.note.value);

    // We simulate the proposal of genesis slot
    let slot_checkpoint =
        th.alice.state.read().await.blockchain.get_slot_checkpoints_by_slot(&[current_slot])?[0]
            .clone()
            .unwrap();

    // With alice's current coin value she can become the slot proposer,
    // so she creates a proposal transaction to burn her staked coin,
    // reward herself and mint the new coin.
    info!(target: "consensus", "[Alice] ====================================");
    info!(target: "consensus", "[Alice] Building proposal transaction params");
    info!(target: "consensus", "[Alice] ====================================");
    let timer = Instant::now();
    let alice_consensus_proposal_call_debris = ConsensusProposalCallBuilder {
        coin: alice_staked_oc.clone(),
        recipient: th.alice.keypair.public,
        slot_checkpoint,
        tree: th.alice.consensus_merkle_tree.clone(),
        burn_zkbin: money_burn_zkbin.clone(),
        burn_pk: money_burn_pk.clone(),
        reward_zkbin: proposal_reward_zkbin.clone(),
        reward_pk: proposal_reward_pk.clone(),
        mint_zkbin: proposal_mint_zkbin.clone(),
        mint_pk: proposal_mint_pk.clone(),
    }
    .build()?;
    let (
        alice_consensus_burn_params,
        alice_consensus_burn_proofs,
        alice_consensus_reward_params,
        alice_consensus_reward_proofs,
        alice_consensus_mint_params,
        alice_consensus_mint_proofs,
        alice_consensus_proposal_secret_key,
    ) = (
        alice_consensus_proposal_call_debris.burn_params,
        alice_consensus_proposal_call_debris.burn_proofs,
        alice_consensus_proposal_call_debris.reward_params,
        alice_consensus_proposal_call_debris.reward_proofs,
        alice_consensus_proposal_call_debris.mint_params,
        alice_consensus_proposal_call_debris.mint_proofs,
        alice_consensus_proposal_call_debris.signature_secret,
    );

    info!(target: "consensus", "[Alice] ====================");
    info!(target: "consensus", "[Alice] Building proposal tx");
    info!(target: "consensus", "[Alice] ====================");
    let mut data = vec![ConsensusFunction::ProposalBurnV1 as u8];
    alice_consensus_burn_params.encode(&mut data)?;
    let consensus_burn_call = ContractCall { contract_id: *CONSENSUS_CONTRACT_ID, data };

    let mut data = vec![ConsensusFunction::ProposalRewardV1 as u8];
    alice_consensus_reward_params.encode(&mut data)?;
    let consensus_reward_call = ContractCall { contract_id: *CONSENSUS_CONTRACT_ID, data };

    let mut data = vec![ConsensusFunction::ProposalMintV1 as u8];
    alice_consensus_mint_params.encode(&mut data)?;
    let consensus_mint_call = ContractCall { contract_id: *CONSENSUS_CONTRACT_ID, data };

    let calls = vec![consensus_burn_call, consensus_reward_call, consensus_mint_call];
    let proofs = vec![
        alice_consensus_burn_proofs,
        alice_consensus_reward_proofs,
        alice_consensus_mint_proofs,
    ];
    let mut alice_proposal_tx = Transaction { calls, proofs, signatures: vec![] };
    let consensus_burn_sigs =
        alice_proposal_tx.create_sigs(&mut OsRng, &[alice_consensus_proposal_secret_key])?;
    let consensus_reward_sigs =
        alice_proposal_tx.create_sigs(&mut OsRng, &[alice_consensus_proposal_secret_key])?;
    let consensus_mint_sigs =
        alice_proposal_tx.create_sigs(&mut OsRng, &[alice_consensus_proposal_secret_key])?;
    alice_proposal_tx.signatures =
        vec![consensus_burn_sigs, consensus_reward_sigs, consensus_mint_sigs];
    proposal_creation_times.push(timer.elapsed());

    // Calculate transaction sizes
    let encoded: Vec<u8> = serialize(&alice_proposal_tx);
    let size = ::std::mem::size_of_val(&*encoded);
    proposal_sizes.push(size);
    let base58 = bs58::encode(&encoded).into_string();
    let size = ::std::mem::size_of_val(&*base58);
    proposal_broadcasted_sizes.push(size);

    info!(target: "consensus", "[Faucet] ===========================");
    info!(target: "consensus", "[Faucet] Executing Alice proposal tx");
    info!(target: "consensus", "[Faucet] ===========================");
    let timer = Instant::now();
    let erroneous_txs = th
        .faucet
        .state
        .read()
        .await
        .verify_transactions(&[alice_proposal_tx.clone()], current_slot, true)
        .await?;
    assert!(erroneous_txs.is_empty());
    th.faucet
        .consensus_merkle_tree
        .append(&MerkleNode::from(alice_consensus_mint_params.output.coin.inner()));
    proposal_verify_times.push(timer.elapsed());

    info!(target: "consensus", "[Alice] ===========================");
    info!(target: "consensus", "[Alice] Executing Alice proposal tx");
    info!(target: "consensus", "[Alice] ===========================");
    let timer = Instant::now();
    let erroneous_txs = th
        .alice
        .state
        .read()
        .await
        .verify_transactions(&[alice_proposal_tx.clone()], current_slot, true)
        .await?;
    assert!(erroneous_txs.is_empty());
    th.alice
        .consensus_merkle_tree
        .append(&MerkleNode::from(alice_consensus_mint_params.output.coin.inner()));
    proposal_verify_times.push(timer.elapsed());

    assert!(
        th.faucet.consensus_merkle_tree.root(0).unwrap() ==
            th.alice.consensus_merkle_tree.root(0).unwrap()
    );

    // Gather new staked owncoin which includes the reward
    let leaf_position = th.alice.consensus_merkle_tree.witness().unwrap();
    let note: MoneyNote =
        alice_consensus_mint_params.output.note.decrypt(&th.alice.keypair.secret)?;
    let alice_rewarded_staked_oc = OwnCoin {
        coin: Coin::from(alice_consensus_mint_params.output.coin),
        note: note.clone(),
        secret: th.alice.keypair.secret,
        nullifier: Nullifier::from(poseidon_hash([th.alice.keypair.secret.inner(), note.serial])),
        leaf_position,
    };

    // Verify values match
    assert!((alice_staked_oc.note.value + REWARD) == alice_rewarded_staked_oc.note.value);

    // Now Alice can unstake her owncoin
    info!(target: "consensus", "[Alice] ==================================");
    info!(target: "consensus", "[Alice] Building Consensus::Unstake params");
    info!(target: "consensus", "[Alice] ==================================");
    let timer = Instant::now();
    let alice_consensus_unstake_call_debris = ConsensusUnstakeCallBuilder {
        coin: alice_rewarded_staked_oc.clone(),
        tree: th.alice.consensus_merkle_tree.clone(),
        burn_zkbin: money_burn_zkbin.clone(),
        burn_pk: money_burn_pk.clone(),
    }
    .build()?;
    let (
        alice_consensus_unstake_params,
        alice_consensus_unstake_proofs,
        alice_consensus_unstake_secret_key,
        alice_consensus_unstake_value_blind,
    ) = (
        alice_consensus_unstake_call_debris.params,
        alice_consensus_unstake_call_debris.proofs,
        alice_consensus_unstake_call_debris.signature_secret,
        alice_consensus_unstake_call_debris.value_blind,
    );

    info!(target: "consensus", "[Alice] ==============================");
    info!(target: "consensus", "[Alice] Building Money::Unstake params");
    info!(target: "consensus", "[Alice] ==============================");
    let alice_money_unstake_call_debris = MoneyUnstakeCallBuilder {
        coin: alice_rewarded_staked_oc.clone(),
        recipient: th.alice.keypair.public,
        value_blind: alice_consensus_unstake_value_blind,
        token_blind: alice_consensus_unstake_params.token_blind,
        nullifier: alice_consensus_unstake_params.input.nullifier,
        merkle_root: alice_consensus_unstake_params.input.merkle_root,
        signature_public: alice_consensus_unstake_params.input.signature_public,
        mint_zkbin: money_mint_zkbin.clone(),
        mint_pk: money_mint_pk.clone(),
    }
    .build()?;
    let (alice_money_unstake_params, alice_money_unstake_proofs) =
        (alice_money_unstake_call_debris.params, alice_money_unstake_call_debris.proofs);

    info!(target: "consensus", "[Alice] ===================");
    info!(target: "consensus", "[Alice] Building unstake tx");
    info!(target: "consensus", "[Alice] ===================");
    let mut data = vec![ConsensusFunction::UnstakeV1 as u8];
    alice_consensus_unstake_params.encode(&mut data)?;
    let consensus_call = ContractCall { contract_id: *CONSENSUS_CONTRACT_ID, data };

    let mut data = vec![MoneyFunction::UnstakeV1 as u8];
    alice_money_unstake_params.encode(&mut data)?;
    let money_call = ContractCall { contract_id: *MONEY_CONTRACT_ID, data };

    let calls = vec![consensus_call, money_call];
    let proofs = vec![alice_consensus_unstake_proofs, alice_money_unstake_proofs];
    let mut alice_unstake_tx = Transaction { calls, proofs, signatures: vec![] };
    let consensus_sigs =
        alice_unstake_tx.create_sigs(&mut OsRng, &[alice_consensus_unstake_secret_key])?;
    let money_sigs =
        alice_unstake_tx.create_sigs(&mut OsRng, &[alice_consensus_unstake_secret_key])?;
    alice_unstake_tx.signatures = vec![consensus_sigs, money_sigs];
    unstake_creation_times.push(timer.elapsed());

    // Calculate transaction sizes
    let encoded: Vec<u8> = serialize(&alice_unstake_tx);
    let size = ::std::mem::size_of_val(&*encoded);
    unstake_sizes.push(size);
    let base58 = bs58::encode(&encoded).into_string();
    let size = ::std::mem::size_of_val(&*base58);
    unstake_broadcasted_sizes.push(size);

    info!(target: "consensus", "[Faucet] ==========================");
    info!(target: "consensus", "[Faucet] Executing Alice unstake tx");
    info!(target: "consensus", "[Faucet] ==========================");
    let timer = Instant::now();
    let erroneous_txs = th
        .faucet
        .state
        .read()
        .await
        .verify_transactions(&[alice_unstake_tx.clone()], current_slot, true)
        .await?;
    assert!(erroneous_txs.is_empty());
    th.faucet.merkle_tree.append(&MerkleNode::from(alice_money_unstake_params.output.coin.inner()));
    unstake_verify_times.push(timer.elapsed());

    info!(target: "consensus", "[Alice] ==========================");
    info!(target: "consensus", "[Alice] Executing Alice unstake tx");
    info!(target: "consensus", "[Alice] ==========================");
    let timer = Instant::now();
    let erroneous_txs = th
        .alice
        .state
        .read()
        .await
        .verify_transactions(&[alice_unstake_tx.clone()], current_slot, true)
        .await?;
    assert!(erroneous_txs.is_empty());
    th.alice.merkle_tree.append(&MerkleNode::from(alice_money_unstake_params.output.coin.inner()));
    unstake_verify_times.push(timer.elapsed());

    assert!(th.faucet.merkle_tree.root(0).unwrap() == th.alice.merkle_tree.root(0).unwrap());
    assert!(
        th.faucet.consensus_merkle_tree.root(0).unwrap() ==
            th.alice.consensus_merkle_tree.root(0).unwrap()
    );

    // Gather new unstaked owncoin
    let leaf_position = th.alice.merkle_tree.witness().unwrap();
    let note: MoneyNote =
        alice_money_unstake_params.output.note.decrypt(&th.alice.keypair.secret)?;
    let alice_unstaked_oc = OwnCoin {
        coin: Coin::from(alice_money_unstake_params.output.coin),
        note: note.clone(),
        secret: th.alice.keypair.secret,
        nullifier: Nullifier::from(poseidon_hash([th.alice.keypair.secret.inner(), note.serial])),
        leaf_position,
    };

    // Verify values match
    assert!(alice_rewarded_staked_oc.note.value == alice_unstaked_oc.note.value);

    // Statistics
    let stake_avg = stake_sizes.iter().sum::<usize>();
    let stake_avg = stake_avg / stake_sizes.len();
    info!("Average Stake size: {:?} Bytes", stake_avg);
    let stake_avg = stake_broadcasted_sizes.iter().sum::<usize>();
    let stake_avg = stake_avg / stake_broadcasted_sizes.len();
    info!("Average Stake broadcasted size: {:?} Bytes", stake_avg);
    let stake_avg = stake_creation_times.iter().sum::<Duration>();
    let stake_avg = stake_avg / stake_creation_times.len() as u32;
    info!("Average Stake creation time: {:?}", stake_avg);
    let stake_avg = stake_verify_times.iter().sum::<Duration>();
    let stake_avg = stake_avg / stake_verify_times.len() as u32;
    info!("Average Stake verification time: {:?}", stake_avg);

    let proposal_avg = proposal_sizes.iter().sum::<usize>();
    let proposal_avg = proposal_avg / proposal_sizes.len();
    info!("Average Proposal size: {:?} Bytes", proposal_avg);
    let proposal_avg = proposal_broadcasted_sizes.iter().sum::<usize>();
    let proposal_avg = proposal_avg / proposal_broadcasted_sizes.len();
    info!("Average Proposal broadcasted size: {:?} Bytes", proposal_avg);
    let proposal_avg = proposal_creation_times.iter().sum::<Duration>();
    let proposal_avg = proposal_avg / proposal_creation_times.len() as u32;
    info!("Average Proposal creation time: {:?}", proposal_avg);
    let proposal_avg = proposal_verify_times.iter().sum::<Duration>();
    let proposal_avg = proposal_avg / proposal_verify_times.len() as u32;
    info!("Average Proposal verification time: {:?}", proposal_avg);

    let unstake_avg = unstake_sizes.iter().sum::<usize>();
    let unstake_avg = unstake_avg / unstake_sizes.len();
    info!("Average Unstake size: {:?} Bytes", unstake_avg);
    let unstake_avg = unstake_broadcasted_sizes.iter().sum::<usize>();
    let unstake_avg = unstake_avg / unstake_broadcasted_sizes.len();
    info!("Average Unstake broadcasted size: {:?} Bytes", unstake_avg);
    let unstake_avg = unstake_creation_times.iter().sum::<Duration>();
    let unstake_avg = unstake_avg / unstake_creation_times.len() as u32;
    info!("Average Unstake creation time: {:?}", unstake_avg);
    let unstake_avg = unstake_verify_times.iter().sum::<Duration>();
    let unstake_avg = unstake_avg / unstake_verify_times.len() as u32;
    info!("Average Unstake verification time: {:?}", unstake_avg);

    // Thanks for reading
    Ok(())
}
