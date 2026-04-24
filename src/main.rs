use alloy::primitives::{Address, U256};
use alloy::providers::layers::CallBatchLayer;
use alloy::providers::{Provider, ProviderBuilder};
use alloy::sol;
use futures::future::LocalBoxFuture;
use futures::stream::StreamExt;
use futures::FutureExt;
use pool_scanner::{
    FactoryConfig, Output, PoolConfig, PoolTypeConfig, ScannerConfig, TokenConfig, TokenInfo,
};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::time::Duration;
use url::Url;

const CONCURRENT_REQUESTS: usize = 200;
const BATCH_WAIT: Duration = Duration::from_millis(10);
const V3_FEE_TIERS: [u32; 5] = [100, 500, 2500, 3000, 10000];

sol! {
    #[sol(rpc)]
    contract ERC20 {
        function balanceOf(address account) external view returns (uint256);
        function decimals() external view returns (uint8);
    }

    #[sol(rpc)]
    contract IUniswapV2Factory {
        function getPair(address tokenA, address tokenB) external view returns (address pair);
    }

    #[sol(rpc)]
    contract IUniswapV3Factory {
        function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address pool);
    }

    #[sol(rpc)]
    contract IUniswapV1Factory {
        function getExchange(address token) external view returns (address exchange);
    }

    #[sol(rpc)]
    contract IBiswapPair {
        function swapFee() external view returns (uint32);
    }

    #[sol(rpc)]
    contract IMdexFactory {
        function getPairFees(address pair) external view returns (uint256);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeeKind {
    Standard,
    Biswap,
    Mdex,
}

impl FeeKind {
    fn from_dex(dex: &str) -> Self {
        match dex.to_ascii_lowercase().as_str() {
            "biswap" => Self::Biswap,
            "mdex" => Self::Mdex,
            _ => Self::Standard,
        }
    }
}

#[derive(Clone)]
struct PoolMeta {
    dex: String,
    pool_type: PoolTypeConfig,
    fee: Option<u32>,
    token0: Option<Address>,
    token1: Option<Address>,
    factory: Address,
    fee_kind: FeeKind,
}

type PoolFuture<'a> = LocalBoxFuture<'a, (Address, PoolMeta)>;
type BalanceFuture<'a> = LocalBoxFuture<'a, (U256, U256, Address, PoolMeta)>;
type FeeFuture<'a> = LocalBoxFuture<'a, (usize, Option<u64>, Option<u64>)>;

fn load_config(network: &str) -> eyre::Result<ScannerConfig> {
    let path = format!("config/{}.toml", network);
    let content =
        fs::read_to_string(&path).map_err(|e| eyre::eyre!("failed to read {}: {}", path, e))?;
    let config: ScannerConfig = toml::from_str(&content)?;
    eprintln!(
        "Loaded {} ({} tokens, {} factories)",
        path,
        config.tokens.len(),
        config.factories.len()
    );
    Ok(config)
}

fn build_pool_futures<'a, P: Provider + Clone + 'a>(
    provider: &P,
    factories: &[FactoryConfig],
    tokens: &[TokenConfig],
    sorted_pairs: &[(Address, Address)],
) -> Vec<PoolFuture<'a>> {
    let mut futures: Vec<PoolFuture<'a>> = Vec::new();

    for factory in factories {
        let fee_kind = FeeKind::from_dex(&factory.name);
        match factory.factory_type {
            PoolTypeConfig::V1 => {
                futures.reserve(tokens.len());
                for t in tokens {
                    futures.push(v1_future(provider, factory, fee_kind, t.address));
                }
            }
            PoolTypeConfig::V2 => {
                futures.reserve(sorted_pairs.len());
                for &(t0, t1) in sorted_pairs {
                    futures.push(v2_future(provider, factory, fee_kind, t0, t1));
                }
            }
            PoolTypeConfig::V3 => {
                futures.reserve(sorted_pairs.len() * V3_FEE_TIERS.len());
                for &(t0, t1) in sorted_pairs {
                    for &fee in &V3_FEE_TIERS {
                        futures.push(v3_future(provider, factory, fee_kind, t0, t1, fee));
                    }
                }
            }
            PoolTypeConfig::V4 => {
                eprintln!("Skipping V4 factory '{}': not yet supported", factory.name);
            }
        }
    }
    futures
}

fn v1_future<'a, P: Provider + Clone + 'a>(
    provider: &P,
    factory: &FactoryConfig,
    fee_kind: FeeKind,
    token: Address,
) -> PoolFuture<'a> {
    let meta = PoolMeta {
        dex: factory.name.clone(),
        pool_type: PoolTypeConfig::V1,
        fee: None,
        token0: Some(token),
        token1: None,
        factory: factory.address,
        fee_kind,
    };
    let provider = provider.clone();
    let factory_address = factory.address;
    async move {
        let univ1 = IUniswapV1Factory::new(factory_address, provider);
        let pool = univ1
            .getExchange(token)
            .call()
            .await
            .unwrap_or(Address::ZERO);
        (pool, meta)
    }
    .boxed_local()
}

fn v2_future<'a, P: Provider + Clone + 'a>(
    provider: &P,
    factory: &FactoryConfig,
    fee_kind: FeeKind,
    t0: Address,
    t1: Address,
) -> PoolFuture<'a> {
    let meta = PoolMeta {
        dex: factory.name.clone(),
        pool_type: PoolTypeConfig::V2,
        fee: None,
        token0: Some(t0),
        token1: Some(t1),
        factory: factory.address,
        fee_kind,
    };
    let provider = provider.clone();
    let factory_address = factory.address;
    async move {
        let univ2 = IUniswapV2Factory::new(factory_address, provider);
        let pool = univ2.getPair(t0, t1).call().await.unwrap_or_else(|e| {
            eprintln!("V2 error {} / {}: {:?}", t0, t1, e);
            Address::ZERO
        });
        (pool, meta)
    }
    .boxed_local()
}

fn v3_future<'a, P: Provider + Clone + 'a>(
    provider: &P,
    factory: &FactoryConfig,
    fee_kind: FeeKind,
    t0: Address,
    t1: Address,
    fee: u32,
) -> PoolFuture<'a> {
    let meta = PoolMeta {
        dex: factory.name.clone(),
        pool_type: PoolTypeConfig::V3,
        fee: Some(fee),
        token0: Some(t0),
        token1: Some(t1),
        factory: factory.address,
        fee_kind,
    };
    let fee_uint = alloy::primitives::Uint::<24, 1>::from(fee);
    let provider = provider.clone();
    let factory_address = factory.address;
    async move {
        let univ3 = IUniswapV3Factory::new(factory_address, provider);
        let pool = univ3
            .getPool(t0, t1, fee_uint)
            .call()
            .await
            .unwrap_or_else(|e| {
                eprintln!("V3 error {} / {} fee={}: {:?}", t0, t1, fee, e);
                Address::ZERO
            });
        (pool, meta)
    }
    .boxed_local()
}

fn build_balance_futures<'a, P: Provider + Clone + 'a>(
    provider: &P,
    pools: &[(Address, PoolMeta)],
    liq_tokens: &HashMap<Address, U256>,
) -> Vec<BalanceFuture<'a>> {
    let mut futures: Vec<BalanceFuture<'a>> = Vec::with_capacity(pools.len() * 2);

    for (pool_addr, meta) in pools {
        for token_addr in [meta.token0, meta.token1].into_iter().flatten() {
            let Some(&min_liq) = liq_tokens.get(&token_addr) else {
                continue;
            };
            let pool_addr = *pool_addr;
            let meta = meta.clone();
            let provider = provider.clone();
            futures.push(
                async move {
                    let erc20 = ERC20::new(token_addr, provider);
                    let balance = erc20.balanceOf(pool_addr).call().await.unwrap_or_else(|e| {
                        eprintln!(
                            "balanceOf error token={} pool={}: {:?}",
                            token_addr, pool_addr, e
                        );
                        U256::ZERO
                    });
                    (balance, min_liq, pool_addr, meta)
                }
                .boxed_local(),
            );
        }
    }
    futures
}

fn build_fee_futures<'a, P: Provider + Clone + 'a>(
    provider: &P,
    pools: &[(Address, PoolMeta)],
) -> Vec<FeeFuture<'a>> {
    let mut futures: Vec<FeeFuture<'a>> = Vec::new();

    for (i, (pool_addr, meta)) in pools.iter().enumerate() {
        match meta.fee_kind {
            FeeKind::Biswap => {
                let provider = provider.clone();
                let pool_addr = *pool_addr;
                futures.push(
                    async move {
                        let pair = IBiswapPair::new(pool_addr, provider);
                        let fee = pair.swapFee().call().await.unwrap_or(1) as u64;
                        (i, Some(1000 - fee), Some(1000))
                    }
                    .boxed_local(),
                );
            }
            FeeKind::Mdex => {
                let provider = provider.clone();
                let pool_addr = *pool_addr;
                let factory_addr = meta.factory;
                futures.push(
                    async move {
                        let factory = IMdexFactory::new(factory_addr, provider);
                        let fee = factory
                            .getPairFees(pool_addr)
                            .call()
                            .await
                            .ok()
                            .and_then(|r| u64::try_from(r).ok())
                            .unwrap_or(30);
                        (i, Some(10000 - fee), Some(10000))
                    }
                    .boxed_local(),
                );
            }
            FeeKind::Standard => {}
        }
    }
    futures
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let network = env::var("NETWORK").unwrap_or_else(|_| "eth".to_string());
    let config = load_config(&network)?;

    let url: Url = config.rpc_url.parse()?;
    let provider = ProviderBuilder::new()
        .layer(CallBatchLayer::new().wait(BATCH_WAIT))
        .connect_http(url);

    let sorted_pairs = pool_scanner::generate_pairs(&config.tokens);
    let token_symbol_map: HashMap<Address, String> = config
        .tokens
        .iter()
        .map(|t| (t.address, t.symbol.clone()))
        .collect();

    let pool_futures =
        build_pool_futures(&provider, &config.factories, &config.tokens, &sorted_pairs);
    eprintln!("Pool discovery: {} queries queued", pool_futures.len());

    let pool_addresses: Vec<(Address, PoolMeta)> = futures::stream::iter(pool_futures)
        .buffer_unordered(CONCURRENT_REQUESTS)
        .filter(|(addr, _)| futures::future::ready(*addr != Address::ZERO))
        .collect()
        .await;

    eprintln!(
        "Pool discovery: {} non-zero pools found",
        pool_addresses.len()
    );

    let liq_tokens_map: HashMap<Address, U256> = pool_scanner::liquidity_tokens(&config.tokens)
        .map_err(|e| eyre::eyre!(e))?
        .into_iter()
        .collect();

    let balance_futures = build_balance_futures(&provider, &pool_addresses, &liq_tokens_map);
    eprintln!(
        "Liquidity check: {} balance queries queued",
        balance_futures.len()
    );

    let mut valid_pools: Vec<(Address, PoolMeta)> = futures::stream::iter(balance_futures)
        .buffer_unordered(CONCURRENT_REQUESTS)
        .filter_map(|(balance, min_liq, pool, meta)| {
            futures::future::ready((balance > min_liq).then_some((pool, meta)))
        })
        .collect()
        .await;

    eprintln!(
        "Liquidity check: {} pools passed (before dedup)",
        valid_pools.len()
    );

    valid_pools.sort_by_key(|(addr, _)| *addr);
    valid_pools.dedup_by_key(|(addr, _)| *addr);

    let fee_futures = build_fee_futures(&provider, &valid_pools);
    let fee_map: HashMap<usize, (Option<u64>, Option<u64>)> = futures::stream::iter(fee_futures)
        .buffer_unordered(CONCURRENT_REQUESTS)
        .map(|(i, num, den)| (i, (num, den)))
        .collect()
        .await;

    let final_pools: Vec<PoolConfig> = valid_pools
        .into_iter()
        .enumerate()
        .map(|(i, (pool_addr, meta))| {
            let (fee_numerator, fee_denominator) = fee_map.get(&i).copied().unwrap_or((None, None));
            PoolConfig {
                pair: pool_addr,
                dex: meta.dex,
                pool_type: meta.pool_type,
                token0: meta
                    .token0
                    .map(|a| TokenInfo::from_address(a, &token_symbol_map)),
                token1: meta
                    .token1
                    .map(|a| TokenInfo::from_address(a, &token_symbol_map)),
                fee_numerator,
                fee_denominator,
                fee: meta.fee,
            }
        })
        .collect();

    let output = Output { pools: final_pools };
    let toml_string = toml::to_string_pretty(&output)?;
    let output_filename = format!("{}_output.toml", network);
    fs::write(&output_filename, &toml_string)?;

    println!("Results written to {}", output_filename);
    println!("Found {} pools", output.pools.len());

    Ok(())
}
