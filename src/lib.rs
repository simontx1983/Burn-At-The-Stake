
use cosmwasm_std::{
    to_json_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, StdError,
    Addr, Uint128, Timestamp, Order, WasmMsg
};
use cosmwasm_schema::schemars::JsonSchema;
use cw20::Cw20ExecuteMsg;
use cw721::Cw721ReceiveMsg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use cw_storage_plus::{Item, Map};

const MIN_STAKING_DAYS: u64 = 7; // Minimum 7 days staking requirement
const SECONDS_IN_DAY: u64 = 86400; // 24 hours * 60 minutes * 60 seconds

// Represents a staker's information
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Staker {
    pub staked_at: Timestamp,
    pub nft_count: u64,
}

// Map to store staker information
const STAKERS: Map<String, Staker> = Map::new("stakers");

// State structure
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct State {
    pub total_staked: u64,
    pub current_pot: Uint128,
    pub last_winner: Option<String>,
    pub stakers: HashSet<String>,
}

// Config structure for immutable contract settings
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub admin: Addr,
    pub nft_contract: Addr,
    pub reward_token: Addr,
}

const CONFIG: Item<Config> = Item::new("config");
const STATE: Item<State> = Item::new("state");

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InstantiateMsg {
    pub admin: String,
    pub nft_contract: String,
    pub reward_token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub enum ExecuteMsg {
    Stake {},
    Unstake {},
    DrawWinner {},
    ClaimReward {},
    // Add message to fund the pot
    FundPot {},
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub enum QueryMsg {
    GetEligibleStakers {},
    GetState {},
    GetStaker { address: String },
}

pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response> {
    let config = Config {
        admin: deps.api.addr_validate(&msg.admin)?,
        nft_contract: deps.api.addr_validate(&msg.nft_contract)?,
        reward_token: deps.api.addr_validate(&msg.reward_token)?,
    };
    
    CONFIG.save(deps.storage, &config)?;
    
    let state = State {
        total_staked: 0,
        current_pot: Uint128::zero(),
        last_winner: None,
        stakers: HashSet::new(),
    };
    STATE.save(deps.storage, &state)?;
    
    Ok(Response::new())
}

pub fn execute_draw_winner(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.admin {
        return Err(StdError::generic_err("Unauthorized"));
    }
    
    let mut state = STATE.load(deps.storage)?;
    if state.stakers.is_empty() {
        return Err(StdError::generic_err("No stakers to draw from"));
    }
    
    // Use Cosmos SDK pseudo-randomness
    let random_bytes = deps.api.random(&env.block.time.nanos().to_be_bytes())?;
    let random_index = random_bytes[0] as usize % state.stakers.len();
    let winner = state.stakers.iter().nth(random_index).unwrap().clone();
    
    state.last_winner = Some(winner.clone());
    // Pot is reset after draw
    let prize = state.current_pot;
    state.current_pot = Uint128::zero();
    
    STATE.save(deps.storage, &state)?;
    
    Ok(Response::new()
        .add_attribute("action", "draw_winner")
        .add_attribute("winner", &winner)
        .add_attribute("prize", prize))
}

pub fn execute_stake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> StdResult<Response> {
    let mut state = STATE.load(deps.storage)?;
    
    // Get or create staker info
    let mut staker = STAKERS.may_load(deps.storage, info.sender.to_string())?
        .unwrap_or(Staker {
            staked_at: env.block.time,
            nft_count: 0,
        });
    
    // Update staker info
    staker.nft_count += 1;
    STAKERS.save(deps.storage, info.sender.to_string(), &staker)?;
    
    // Update state
    state.stakers.insert(info.sender.to_string());
    state.total_staked += 1;
    STATE.save(deps.storage, &state)?;
    
    Ok(Response::new()
        .add_attribute("action", "stake")
        .add_attribute("sender", info.sender))
}

pub fn execute_unstake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> StdResult<Response> {
    let mut state = STATE.load(deps.storage)?;
    
    // Get staker info
    let staker = STAKERS.may_load(deps.storage, info.sender.to_string())?;
    if staker.is_none() {
        return Err(StdError::generic_err("Not staked"));
    }
    let mut staker = staker.unwrap();
    
    // Check minimum staking requirement
    let time_diff = env.block.time.seconds() - staker.staked_at.seconds();
    if time_diff < MIN_STAKING_DAYS * SECONDS_IN_DAY {
        return Err(StdError::generic_err("Minimum staking requirement not met"));
    }
    
    // Update staker info
    staker.nft_count -= 1;
    if staker.nft_count == 0 {
        STAKERS.remove(deps.storage, info.sender.to_string());
    } else {
        STAKERS.save(deps.storage, info.sender.to_string(), &staker)?;
    }
    
    // Update state
    state.stakers.remove(&info.sender.to_string());
    state.total_staked -= 1;
    STATE.save(deps.storage, &state)?;
    
    Ok(Response::new()
        .add_attribute("action", "unstake")
        .add_attribute("sender", info.sender))
}

pub fn execute_claim_reward(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
) -> StdResult<Response> {
    let config = CONFIG.load(deps.storage)?;
    let state = STATE.load(deps.storage)?;
    
    if let Some(last_winner) = &state.last_winner {
        if info.sender.as_str() != last_winner {
            return Err(StdError::generic_err("Not the winner"));
        }
        
        // Create transfer message
        let transfer_msg = Cw20ExecuteMsg::Transfer {
            recipient: info.sender.to_string(),
            amount: state.current_pot,
        };
        
        let msg = WasmMsg::Execute {
            contract_addr: config.reward_token.to_string(),
            msg: to_json_binary(&transfer_msg)?,
            funds: vec![],
        };
        
        Ok(Response::new()
            .add_message(msg)
            .add_attribute("action", "claim_reward")
            .add_attribute("winner", info.sender)
            .add_attribute("amount", state.current_pot))
    } else {
        Err(StdError::generic_err("No winner to claim"))
    }
}

pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetEligibleStakers {} => to_json_binary(&query_eligible_stakers(deps, env)?),
        QueryMsg::GetState {} => to_json_binary(&query_state(deps)?),
        QueryMsg::GetStaker { address } => to_json_binary(&query_staker(deps, address)?),
    }
}

fn query_eligible_stakers(deps: Deps, env: Env) -> StdResult<Vec<(String, Staker)>> {
    let mut eligible_stakers = Vec::new();
    
    // Iterate through all stakers
    STAKERS.range(deps.storage, None, None, Order::Ascending)
        .filter_map(|item| item.ok())
        .for_each(|(address, staker)| {
            // Check if staker has met minimum staking requirement
            if staker.staked_at.plus_seconds(MIN_STAKING_DAYS * SECONDS_IN_DAY) <= env.block.time {
                eligible_stakers.push((address.to_string(), staker));
            }
        });
    
    Ok(eligible_stakers)
}

// Add helper function to get total staked NFTs for DAO DAO
pub fn get_total_staked_nfts(deps: Deps) -> StdResult<u64> {
    let state = STATE.load(deps.storage)?;
    Ok(state.total_staked)
}

// Add helper function to get staker weight for DAO DAO
pub fn get_staker_weight(deps: Deps, address: String) -> StdResult<u64> {
    let staker = STAKERS.may_load(deps.storage, &address)?;
    Ok(staker.map_or(0, |s| s.nft_count))
}

fn query_state(deps: Deps) -> StdResult<State> {
    let state = STATE.load(deps.storage)?;
    Ok(state)
}

fn query_staker(deps: Deps, address: String) -> StdResult<Option<Staker>> {
    let staker = STAKERS.may_load(deps.storage, &address)?;
    Ok(staker)
}