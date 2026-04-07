use alloy::primitives::{Address, U256};
use alloy::providers::layers::CallBatchLayer;
use alloy::providers::ProviderBuilder;
use alloy::sol;
use futures::future::LocalBoxFuture;
use futures::stream::StreamExt;
use futures::FutureExt;
use pool_scanner::{Output, PoolConfig, PoolTypeConfig, ScannerConfig, TokenInfo};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;
use url::Url;

const CONCURRENT_REQUESTS: usize = 200;
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

#[derive(Clone)]
struct PoolMeta {
    dex: String,
    pool_type: PoolTypeConfig,
    fee: Option<u32>,
    token0: Option<Address>,
    token1: Option<Address>,
    factory: Address,
}

type FeeResult<'a> = LocalBoxFuture<'a, (usize, Option<u64>, Option<u64>)>;

fn load_config(network: &str) -> eyre::Result<ScannerConfig> {
    let config_path = format!("config/{}.toml", network);
    let config_path = if Path::new(&config_path).exists() {
        config_path
    } else {
        format!("pool_scanner/config/{}.toml", network)
    };

    let content = fs::read_to_string(&config_path)
        .unwrap_or_else(|_| panic!("Failed to read config file at {}", config_path));
    let config: ScannerConfig = toml::from_str(&content)?;

    eprintln!("Config path: {}", config_path);
    eprintln!("Tokens: {}", config.tokens.len());

    Ok(config)
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let network = env::var("NETWORK").unwrap_or_else(|_| "eth".to_string());
    let config = load_config(&network)?;

    let url: Url = config.rpc_url.parse()?;
    let provider = ProviderBuilder::new()
        .layer(CallBatchLayer::new().wait(Duration::from_millis(10)))
        .connect_http(url);

    let pairs_to_check = pool_scanner::generate_pairs(&config.tokens);
    let all_addresses: Vec<Address> = config.tokens.iter().map(|t| t.address).collect();
    let token_symbol_map: HashMap<Address, String> = config
        .tokens
        .iter()
        .map(|t| (t.address, t.symbol.clone()))
        .collect();

    let mut pool_futures: Vec<LocalBoxFuture<'_, (Address, PoolMeta)>> = Vec::new();

    for factory in &config.factories {
        match factory.factory_type {
            PoolTypeConfig::V1 => {
                for &token_addr in &all_addresses {
                    let meta = PoolMeta {
                        dex: factory.name.clone(),
                        pool_type: PoolTypeConfig::V1,
                        fee: None,
                        token0: Some(token_addr),
                        token1: None,
                        factory: factory.address,
                    };
                    let provider = provider.clone();
                    let factory_address = factory.address;
                    pool_futures.push(
                        async move {
                            let univ1 = IUniswapV1Factory::new(factory_address, provider);
                            let pool_addr = univ1
                                .getExchange(token_addr)
                                .call()
                                .await
                                .unwrap_or(Address::ZERO);
                            (pool_addr, meta)
                        }
                        .boxed_local(),
                    );
                }
            }
            PoolTypeConfig::V2 => {
                for &(t0, t1) in &pairs_to_check {
                    let (token0, token1) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
                    let meta = PoolMeta {
                        dex: factory.name.clone(),
                        pool_type: PoolTypeConfig::V2,
                        fee: None,
                        token0: Some(token0),
                        token1: Some(token1),
                        factory: factory.address,
                    };
                    let provider = provider.clone();
                    let factory_address = factory.address;
                    pool_futures.push(
                        async move {
                            let univ2 = IUniswapV2Factory::new(factory_address, provider);
                            let pool_addr =
                                univ2.getPair(t0, t1).call().await.unwrap_or_else(|e| {
                                    eprintln!("V2 Error for {} / {}: {:?}", t0, t1, e);
                                    Address::ZERO
                                });
                            (pool_addr, meta)
                        }
                        .boxed_local(),
                    );
                }
            }
            PoolTypeConfig::V3 => {
                for &(t0, t1) in &pairs_to_check {
                    for &fee in &V3_FEE_TIERS {
                        let (token0, token1) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
                        let meta = PoolMeta {
                            dex: factory.name.clone(),
                            pool_type: PoolTypeConfig::V3,
                            fee: Some(fee),
                            token0: Some(token0),
                            token1: Some(token1),
                            factory: factory.address,
                        };
                        let fee_uint = alloy::primitives::Uint::<24, 1>::from(fee);
                        let provider = provider.clone();
                        let factory_address = factory.address;
                        pool_futures.push(
                            async move {
                                let univ3 =
                                    IUniswapV3Factory::new(factory_address, provider);
                                let pool_addr = univ3
                                    .getPool(t0, t1, fee_uint)
                                    .call()
                                    .await
                                    .unwrap_or_else(|e| {
                                        eprintln!(
                                            "V3 Error for {} / {} fee={}: {:?}",
                                            t0, t1, fee, e
                                        );
                                        Address::ZERO
                                    });
                                (pool_addr, meta)
                            }
                            .boxed_local(),
                        );
                    }
                }
            }
            PoolTypeConfig::V4 => {}
        }
    }

    eprintln!("Pool discovery: {} queries queued", pool_futures.len());

    let pool_addresses: Vec<_> = futures::stream::iter(pool_futures)
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter(|(addr, _)| *addr != Address::ZERO)
        .collect();

    eprintln!(
        "Pool discovery: {} non-zero pools found",
        pool_addresses.len()
    );

    let liq_tokens_map: HashMap<Address, U256> = pool_scanner::liquidity_tokens(&config.tokens)
        .into_iter()
        .collect();

    let mut balance_futures: Vec<LocalBoxFuture<'_, (U256, U256, Address, PoolMeta)>> = Vec::new();

    for (pool_addr, meta) in &pool_addresses {
        let tokens_to_check: Vec<Address> =
            [meta.token0, meta.token1].into_iter().flatten().collect();

        for token_addr in tokens_to_check {
            if let Some(&min_liq) = liq_tokens_map.get(&token_addr) {
                let pool_addr = *pool_addr;
                let meta = meta.clone();
                let provider = provider.clone();

                balance_futures.push(
                    async move {
                        let erc20 = ERC20::new(token_addr, provider);
                        let balance =
                            erc20.balanceOf(pool_addr).call().await.unwrap_or_else(|e| {
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
    }

    eprintln!(
        "Liquidity check: {} balance queries queued",
        balance_futures.len()
    );

    let mut valid_pools: Vec<_> = futures::stream::iter(balance_futures)
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter(|(balance, min_liq, _, _)| balance > min_liq)
        .map(|(_, _, pool_addr, meta)| (pool_addr, meta))
        .collect();

    eprintln!(
        "Liquidity check: {} pools passed (before dedup)",
        valid_pools.len()
    );

    valid_pools.sort_by_key(|(addr, _)| *addr);
    valid_pools.dedup_by_key(|(addr, _)| *addr);

    let mut fee_futures: Vec<FeeResult<'_>> = Vec::new();

    for (i, (pool_addr, meta)) in valid_pools.iter().enumerate() {
        let dex_lower = meta.dex.to_lowercase();
        if dex_lower == "biswap" {
            let provider = provider.clone();
            let pool_addr = *pool_addr;
            fee_futures.push(
                async move {
                    let pair = IBiswapPair::new(pool_addr, provider);
                    let fee = pair.swapFee().call().await.unwrap_or(1) as u64;
                    (i, Some(1000 - fee), Some(1000))
                }
                .boxed_local(),
            );
        } else if dex_lower == "mdex" {
            let provider = provider.clone();
            let pool_addr = *pool_addr;
            let factory_addr = meta.factory;
            fee_futures.push(
                async move {
                    let factory = IMdexFactory::new(factory_addr, provider);
                    let fee = factory
                        .getPairFees(pool_addr)
                        .call()
                        .await
                        .map(|r| r.to_string().parse::<u64>().unwrap_or(30))
                        .unwrap_or(30);
                    (i, Some(10000 - fee), Some(10000))
                }
                .boxed_local(),
            );
        }
    }

    let fee_map: HashMap<usize, (Option<u64>, Option<u64>)> =
        futures::stream::iter(fee_futures)
            .buffer_unordered(CONCURRENT_REQUESTS)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|(i, num, den)| (i, (num, den)))
            .collect();

    let final_pools: Vec<PoolConfig> = valid_pools
        .into_iter()
        .enumerate()
        .map(|(i, (pool_addr, meta))| {
            let (fee_numerator, fee_denominator) =
                fee_map.get(&i).cloned().unwrap_or((None, None));

            PoolConfig {
                pair: pool_addr,
                dex: meta.dex,
                pool_type: meta.pool_type,
                token0: meta
                    .token0
                    .map(|addr| TokenInfo::from_address(addr, &token_symbol_map)),
                token1: meta
                    .token1
                    .map(|addr| TokenInfo::from_address(addr, &token_symbol_map)),
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
