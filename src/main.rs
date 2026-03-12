use alloy::primitives::{Address, U256};
use alloy::providers::layers::CallBatchLayer;
use alloy::providers::ProviderBuilder;
use alloy::sol;
use pool_scanner::{deduplicate_pools, Output, PoolConfig, PoolTypeConfig, ScannerConfig};
use std::env;
use std::fs;
use std::path::Path;
use std::time::Duration;
use url::Url;

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

    type PoolMetaTuple = (String, PoolTypeConfig, Option<u32>);
    let mut pool_futures: Vec<
        std::pin::Pin<Box<dyn std::future::Future<Output = (Address, PoolMetaTuple)>>>,
    > = Vec::new();

    let pairs_to_check = pool_scanner::generate_pairs(&config.tokens);
    let all_addresses: Vec<Address> = config.tokens.iter().map(|t| t.address).collect();

    for factory in &config.factories {
        match factory.factory_type {
            PoolTypeConfig::V1 => {
                for &token_addr in &all_addresses {
                    let meta = (factory.name.clone(), PoolTypeConfig::V1, None);
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
                    let meta = (factory.name.clone(), PoolTypeConfig::V2, None);
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
                        let meta = (factory.name.clone(), PoolTypeConfig::V3, Some(fee));
                        let fee_uint = alloy::primitives::Uint::<24, 1>::from(fee);
                        let provider = provider.clone();
                        let factory_address = factory.address;
                        pool_futures.push(Box::pin(async move {
                            let univ3 = IUniswapV3Factory::new(factory_address, provider);
                            let pool_addr = match univ3.getPool(t0, t1, fee_uint).call().await {
                                Ok(r) => r,
                                Err(_) => Address::ZERO,
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

    let results = futures::future::join_all(pool_futures).await;

    let mut pool_addresses = Vec::new();
    for (pool_addr, meta) in results {
        if pool_addr != Address::ZERO {
            pool_addresses.push((pool_addr, meta));
        }
    }

    let liq_tokens = pool_scanner::liquidity_tokens(&config.tokens);

    // (balance, min_liquidity, pool_addr, meta)
    let mut balance_futures: Vec<
        std::pin::Pin<Box<dyn std::future::Future<Output = (U256, U256, Address, PoolMetaTuple)>>>,
    > = Vec::new();

    for (pool_addr, meta) in &pool_addresses {
        for &(token_addr, min_liq) in &liq_tokens {
            let pool_addr_clone = *pool_addr;
            let meta_clone = meta.clone();
            let provider = provider.clone();

            balance_futures.push(Box::pin(async move {
                let erc20 = ERC20::new(token_addr, provider);
                let balance = match erc20.balanceOf(pool_addr_clone).call().await {
                    Ok(r) => r,
                    Err(_) => U256::ZERO,
                };
                (balance, min_liq, pool_addr_clone, meta_clone)
            })
                as std::pin::Pin<Box<dyn std::future::Future<Output = _>>>);
        }
    }

    let balance_results = futures::future::join_all(balance_futures).await;

    let mut valid_pools = Vec::new();
    for (balance, min_liq, pool_addr, meta) in balance_results {
        if balance > min_liq {
            valid_pools.push(PoolConfig {
                pair: pool_addr,
                dex: meta.0,
                pool_type: meta.1,
                fee_numerator: None,
                fee_denominator: None,
                fee: meta.2,
            });
        }
    }

    let final_pools = deduplicate_pools(valid_pools);

    let output = Output { pools: final_pools };
    let toml_string = toml::to_string_pretty(&output)?;

    let output_filename = format!("{}_output.toml", network);
    fs::write(&output_filename, &toml_string)?;

    println!("Results written to {}", output_filename);
    println!("Found {} pools", output.pools.len());

    Ok(())
}
