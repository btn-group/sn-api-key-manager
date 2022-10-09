use crate::constants::{
    BLOCK_SIZE, CONFIG_KEY, MOCK_AMOUNT, MOCK_BUTT_ADDRESS, MOCK_TOKEN_ADDRESS,
    PREFIX_CANCEL_RECORDS, PREFIX_CANCEL_RECORDS_COUNT, PREFIX_FILL_RECORDS_COUNT,
};
use crate::msg::{HandleMsg, InitMsg, QueryMsg};
use crate::state::{ActivityRecord, Config, SecretContract};
use crate::validations::authorize;
use cosmwasm_std::{
    to_binary, Api, Binary, CanonicalAddr, Env, Extern, HandleResponse, HumanAddr, InitResponse,
    Querier, ReadonlyStorage, StdResult, Storage, Uint128,
};
use cosmwasm_storage::{PrefixedStorage, ReadonlyPrefixedStorage};
use primitive_types::U256;
use secret_toolkit::snip20;
use secret_toolkit::storage::{TypedStore, TypedStoreMut};

pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    let mut config_store = TypedStoreMut::attach(&mut deps.storage);
    let config: Config = Config {
        addresses_allowed_to_fill: vec![env.message.sender.clone(), env.contract.address],
        admin: env.message.sender,
        butt: msg.butt,
        execution_fee: msg.execution_fee,
        sscrt: msg.sscrt,
    };
    config_store.store(CONFIG_KEY, &config)?;

    Ok(InitResponse {
        messages: vec![],
        log: vec![],
    })
}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    match msg {
        HandleMsg::Receive {
            from, amount, msg, ..
        } => receive(deps, env, from, amount, msg),
        HandleMsg::UpdateConfig {
            addresses_allowed_to_fill,
            execution_fee,
        } => update_config(deps, &env, addresses_allowed_to_fill, execution_fee),
    }
}

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => {
            let config: Config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;
            Ok(to_binary(&config)?)
        }
    }
}

fn receive<S: Storage, A: Api, Q: Querier>(
    _deps: &mut Extern<S, A, Q>,
    _env: Env,
    _from: HumanAddr,
    _amount: Uint128,
    _msg: Option<Binary>,
) -> StdResult<HandleResponse> {
    let response = Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: None,
    });
    pad_response(response)
}

fn prefix_activity_records_count(activity_records_storage_prefix: &[u8]) -> &[u8] {
    if activity_records_storage_prefix == PREFIX_CANCEL_RECORDS {
        PREFIX_CANCEL_RECORDS_COUNT
    } else {
        PREFIX_FILL_RECORDS_COUNT
    }
}

fn set_count<S: Storage>(
    store: &mut S,
    for_address: &CanonicalAddr,
    storage_prefix: &[u8],
    count: u128,
) -> StdResult<()> {
    let mut prefixed_store = PrefixedStorage::new(storage_prefix, store);
    let mut count_store = TypedStoreMut::<u128, _>::attach(&mut prefixed_store);
    count_store.store(for_address.as_slice(), &count)
}

fn calculate_fee(user_butt_balance: Uint128, to_amount: Uint128) -> Uint128 {
    let user_butt_balance_as_u128: u128 = user_butt_balance.u128();
    let nom = if user_butt_balance_as_u128 >= 100_000_000_000 {
        0
    } else if user_butt_balance_as_u128 >= 50_000_000_000 {
        6
    } else if user_butt_balance_as_u128 >= 25_000_000_000 {
        12
    } else if user_butt_balance_as_u128 >= 12_500_000_000 {
        18
    } else if user_butt_balance_as_u128 >= 6_250_000_000 {
        24
    } else {
        30
    };
    let fee: u128 = if nom == 0 {
        0
    } else {
        (U256::from(to_amount.u128()) * U256::from(nom) / U256::from(10_000)).as_u128()
    };

    Uint128(fee)
}

fn get_activity_records<S: ReadonlyStorage>(
    storage: &S,
    for_address: &CanonicalAddr,
    page: u128,
    page_size: u128,
    storage_prefix: &[u8],
) -> StdResult<(Vec<ActivityRecord>, u128)> {
    let total: u128 = storage_count(
        storage,
        for_address,
        prefix_activity_records_count(storage_prefix),
    )?;
    let offset: u128 = page * page_size;
    let end = total - offset;
    let start = end.saturating_sub(page_size);
    let store =
        ReadonlyPrefixedStorage::multilevel(&[storage_prefix, for_address.as_slice()], storage);
    let mut activity_records: Vec<ActivityRecord> = Vec::new();
    let store = TypedStore::<ActivityRecord, _>::attach(&store);
    for position in (start..end).rev() {
        activity_records.push(store.load(&position.to_le_bytes())?);
    }

    Ok((activity_records, total))
}

fn pad_response(response: StdResult<HandleResponse>) -> StdResult<HandleResponse> {
    response.map(|mut response| {
        response.data = response.data.map(|mut data| {
            space_pad(BLOCK_SIZE, &mut data.0);
            data
        });
        response
    })
}

fn query_balance_of_token<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
    token: SecretContract,
    viewing_key: String,
) -> StdResult<Uint128> {
    if token.address == HumanAddr::from(MOCK_TOKEN_ADDRESS)
        || token.address == HumanAddr::from(MOCK_BUTT_ADDRESS)
    {
        Ok(Uint128(MOCK_AMOUNT))
    } else {
        let balance = snip20::balance_query(
            &deps.querier,
            address,
            viewing_key,
            BLOCK_SIZE,
            token.contract_hash,
            token.address,
        )?;
        Ok(balance.amount)
    }
}

// Take a Vec<u8> and pad it up to a multiple of `block_size`, using spaces at the end.
fn space_pad(block_size: usize, message: &mut Vec<u8>) -> &mut Vec<u8> {
    let len = message.len();
    let surplus = len % block_size;
    if surplus == 0 {
        return message;
    }

    let missing = block_size - surplus;
    message.reserve(missing);
    message.extend(std::iter::repeat(b' ').take(missing));
    message
}

fn update_config<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: &Env,
    addresses_allowed_to_fill: Option<Vec<HumanAddr>>,
    execution_fee: Option<Uint128>,
) -> StdResult<HandleResponse> {
    let mut config_store = TypedStoreMut::attach(&mut deps.storage);
    let mut config: Config = config_store.load(CONFIG_KEY).unwrap();
    authorize(vec![config.admin.clone()], &env.message.sender)?;

    if let Some(addresses_allowed_to_fill_unwrapped) = addresses_allowed_to_fill {
        config.addresses_allowed_to_fill = addresses_allowed_to_fill_unwrapped;
        if !config
            .addresses_allowed_to_fill
            .contains(&env.contract.address)
        {
            config
                .addresses_allowed_to_fill
                .push(env.contract.address.clone())
        }
        if !config
            .addresses_allowed_to_fill
            .contains(&config.admin.clone())
        {
            config.addresses_allowed_to_fill.push(config.admin.clone())
        }
    }
    if let Some(execution_fee_unwrapped) = execution_fee {
        config.execution_fee = execution_fee_unwrapped;
    }
    config_store.store(CONFIG_KEY, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: None,
    })
}
