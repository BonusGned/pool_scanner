use alloy::primitives::{Address, U256};
use alloy::providers::layers::CallBatchLayer;
use alloy::providers::ProviderBuilder;
use alloy::sol;
use futures::stream::StreamExt;
use pool_scanner::{Output, PoolConfig, PoolTypeConfig, ScannerConfig, TokenInfo};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;
use url::Url;

const CONCURRENT_REQUESTS: usize = 200;

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

fn token_info_from_address(addr: Address, symbols: &HashMap<Address, String>) -> TokenInfo {
    let symbol = symbols
        .get(&addr)
        .cloned()
        .unwrap_or_else(|| addr.to_string());
    TokenInfo {
        address: addr,
        symbol,
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let network = env::var("NETWORK").unwrap_or_else(|_| "eth".to_string());
    let config_path = format!("config/{}.toml", network);

    let config_path = if Path::new(&config_path).exists() {
        config_path
    } else {
        format!("pool_scanner/config/{}.toml", network)
    };

    let config_content = fs::read_to_string(&config_path)
        .unwrap_or_else(|_| panic!("Failed to read config file at {}", config_path));
    let config: ScannerConfig = toml::from_str(&config_content)?;

    eprintln!("Config path: {}", config_path);
    eprintln!("Tokens: {}", config.tokens.len());

    let url: Url = config.rpc_url.parse()?;

    let provider = ProviderBuilder::new()
        .layer(CallBatchLayer::new().wait(Duration::from_millis(10)))
        .connect_http(url);

    type PoolFuture = std::pin::Pin<Box<dyn std::future::Future<Output = (Address, PoolMeta)>>>;
    type BalanceFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = (U256, U256, Address, PoolMeta)>>>;

    let mut pool_futures: Vec<PoolFuture> = Vec::new();

    let pairs_to_check = pool_scanner::generate_pairs(&config.tokens);
    let all_addresses: Vec<Address> = config.tokens.iter().map(|t| t.address).collect();
    let token_symbol_map: HashMap<Address, String> = config
        .tokens
        .iter()
        .map(|t| (t.address, t.symbol.clone()))
        .collect();

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
                    pool_futures.push(Box::pin(async move {
                        let univ1 = IUniswapV1Factory::new(factory_address, provider);
                        let pool_addr = match univ1.getExchange(token_addr).call().await {
                            Ok(r) => r,
                            Err(_) => Address::ZERO,
                        };
                        (pool_addr, meta)
                    })
                        as std::pin::Pin<Box<dyn std::future::Future<Output = _>>>);
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
                    pool_futures.push(Box::pin(async move {
                        let univ2 = IUniswapV2Factory::new(factory_address, provider);
                        let pool_addr = match univ2.getPair(t0, t1).call().await {
                            Ok(r) => r,
                            Err(e) => {
                                eprintln!("V2 Error for {} / {}: {:?}", t0, t1, e);
                                Address::ZERO
                            }
                        };
                        (pool_addr, meta)
                    })
                        as std::pin::Pin<Box<dyn std::future::Future<Output = _>>>);
                }
            }
            PoolTypeConfig::V3 => {
                let v3_fees = vec![100u32, 500u32, 2500u32, 3000u32, 10000u32];
                for &(t0, t1) in &pairs_to_check {
                    for &fee in &v3_fees {
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
                        pool_futures.push(Box::pin(async move {
                            let univ3 = IUniswapV3Factory::new(factory_address, provider);
                            let pool_addr = match univ3.getPool(t0, t1, fee_uint).call().await {
                                Ok(r) => r,
                                Err(e) => {
                                    eprintln!("V3 Error for {} / {} fee={}: {:?}", t0, t1, fee, e);
                                    Address::ZERO
                                }
                            };
                            (pool_addr, meta)
                        })
                            as std::pin::Pin<Box<dyn std::future::Future<Output = _>>>);
                    }
                }
            }
            PoolTypeConfig::V4 => {}
        }
    }

    eprintln!("Pool discovery: {} queries queued", pool_futures.len());

    let results: Vec<_> = futures::stream::iter(pool_futures)
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect()
        .await;

    let mut pool_addresses = Vec::new();
    for (pool_addr, meta) in results {
        if pool_addr != Address::ZERO {
            pool_addresses.push((pool_addr, meta));
        }
    }

    eprintln!(
        "Pool discovery: {} non-zero pools found",
        pool_addresses.len()
    );

    let mut liq_tokens_map = HashMap::new();
    for (addr, min_liq) in pool_scanner::liquidity_tokens(&config.tokens) {
        liq_tokens_map.insert(addr, min_liq);
    }

    // (balance, min_liquidity, pool_addr, meta)
    let mut balance_futures: Vec<BalanceFuture> = Vec::new();

    for (pool_addr, meta) in &pool_addresses {
        let mut tokens_to_check = Vec::new();
        if let Some(t0) = meta.token0 {
            tokens_to_check.push(t0);
        }
        if let Some(t1) = meta.token1 {
            tokens_to_check.push(t1);
        }

        for token_addr in tokens_to_check {
            if let Some(&min_liq) = liq_tokens_map.get(&token_addr) {
                let pool_addr_clone = *pool_addr;
                let meta_clone = meta.clone();
                let provider = provider.clone();

                balance_futures.push(Box::pin(async move {
                    let erc20 = ERC20::new(token_addr, provider);
                    let balance = match erc20.balanceOf(pool_addr_clone).call().await {
                        Ok(r) => r,
                        Err(e) => {
                            eprintln!(
                                "balanceOf error token={} pool={}: {:?}",
                                token_addr, pool_addr_clone, e
                            );
                            U256::ZERO
                        }
                    };
                    (balance, min_liq, pool_addr_clone, meta_clone)
                })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = _>>>);
            }
        }
    }

    eprintln!(
        "Liquidity check: {} balance queries queued",
        balance_futures.len()
    );

    let balance_results: Vec<_> = futures::stream::iter(balance_futures)
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect()
        .await;

    let mut valid_pools_with_meta = Vec::new();
    for (balance, min_liq, pool_addr, meta) in balance_results {
        if balance > min_liq {
            valid_pools_with_meta.push((pool_addr, meta));
        }
    }

    eprintln!(
        "Liquidity check: {} pools passed (before dedup)",
        valid_pools_with_meta.len()
    );

    valid_pools_with_meta.sort_by_key(|(addr, _)| *addr);
    valid_pools_with_meta.dedup_by_key(|(addr, _)| *addr);

    type FeeFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = (usize, Option<u64>, Option<u64>)>>>;
    let mut fee_futures: Vec<FeeFuture> = Vec::new();

    for (i, (pool_addr, meta)) in valid_pools_with_meta.iter().enumerate() {
        let dex_lower = meta.dex.to_lowercase();
        if dex_lower == "biswap" {
            let provider = provider.clone();
            let pool_addr = *pool_addr;
            fee_futures.push(Box::pin(async move {
                let pair = IBiswapPair::new(pool_addr, provider);
                let fee = match pair.swapFee().call().await {
                    Ok(r) => r as u64,
                    Err(_) => 1, // default 0.1%
                };
                (i, Some(1000 - fee), Some(1000))
            }));
        } else if dex_lower == "mdex" {
            let provider = provider.clone();
            let pool_addr = *pool_addr;
            let factory_addr = meta.factory;
            fee_futures.push(Box::pin(async move {
                let factory = IMdexFactory::new(factory_addr, provider);
                let fee = match factory.getPairFees(pool_addr).call().await {
                    Ok(r) => {
                        let fee_str = r.to_string();
                        fee_str.parse::<u64>().unwrap_or(30)
                    }
                    Err(_) => 30, // default 0.3%
                };
                (i, Some(10000 - fee), Some(10000))
            }));
        }
    }

    let fee_results: Vec<_> = futures::stream::iter(fee_futures)
        .buffer_unordered(CONCURRENT_REQUESTS)
        .collect()
        .await;

    let mut fee_map = HashMap::new();
    for (i, num, den) in fee_results {
        fee_map.insert(i, (num, den));
    }

    let mut final_pools = Vec::new();
    for (i, (pool_addr, meta)) in valid_pools_with_meta.into_iter().enumerate() {
        let (fee_numerator, fee_denominator) = fee_map.get(&i).cloned().unwrap_or((None, None));

        final_pools.push(PoolConfig {
            pair: pool_addr,
            dex: meta.dex,
            pool_type: meta.pool_type,
            token0: meta
                .token0
                .map(|addr| token_info_from_address(addr, &token_symbol_map)),
            token1: meta
                .token1
                .map(|addr| token_info_from_address(addr, &token_symbol_map)),
            fee_numerator,
            fee_denominator,
            fee: meta.fee,
        });
    }

    let output = Output { pools: final_pools };
    let toml_string = toml::to_string_pretty(&output)?;

    let output_filename = format!("{}_output.toml", network);
    fs::write(&output_filename, &toml_string)?;

    println!("Results written to {}", output_filename);
    println!("Found {} pools", output.pools.len());

    Ok(())
}
