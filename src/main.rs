use alloy::primitives::{Address, U256};
use alloy::providers::layers::CallBatchLayer;
use alloy::providers::ProviderBuilder;
use alloy::sol;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PoolTypeConfig {
    V1,
    V2,
    V3,
    V4,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryConfig {
    pub name: String,
    pub address: Address,
    #[serde(rename = "type")]
    pub factory_type: PoolTypeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    pub rpc_url: String,
    pub multicall3_address: Address,
    pub stables: Vec<Address>,
    pub other_tokens: Vec<Address>,
    pub factories: Vec<FactoryConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PoolConfig {
    pub pair: Address,
    pub dex: String,
    pub pool_type: PoolTypeConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_numerator: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_denominator: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Output {
    pub pools: Vec<PoolConfig>,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let network = env::var("NETWORK").unwrap_or_else(|_| "eth".to_string());
    let config_path = format!("config/{}.toml", network);

    // Fallback if running from the root of the workspace
    let config_path = if Path::new(&config_path).exists() {
        config_path
    } else {
        format!("pool_scanner/config/{}.toml", network)
    };

    let config_content = fs::read_to_string(&config_path)
        .unwrap_or_else(|_| panic!("Failed to read config file at {}", config_path));
    let config: ScannerConfig = toml::from_str(&config_content)?;

    let url: Url = config.rpc_url.parse()?;

    // Setup provider with CallBatchLayer
    let provider = ProviderBuilder::new()
        .layer(CallBatchLayer::new().wait(Duration::from_millis(10)))
        .connect_http(url);

    type PoolMeta = (String, PoolTypeConfig, Option<u32>);
    let mut pool_futures: Vec<
        std::pin::Pin<Box<dyn std::future::Future<Output = (Address, PoolMeta)>>>,
    > = Vec::new();

    let mut pairs_to_check = Vec::new();
    // 1. Stable vs Other
    for &stable in &config.stables {
        for &other in &config.other_tokens {
            pairs_to_check.push((stable, other));
        }
    }
    // 2. Stable vs Stable
    for i in 0..config.stables.len() {
        for j in (i + 1)..config.stables.len() {
            pairs_to_check.push((config.stables[i], config.stables[j]));
        }
    }

    // Build calls
    for factory in &config.factories {
        match factory.factory_type {
            PoolTypeConfig::V1 => {
                for &other in &config.other_tokens {
                    let meta = (factory.name.clone(), PoolTypeConfig::V1, None);
                    let provider = provider.clone();
                    let factory_address = factory.address;
                    pool_futures.push(Box::pin(async move {
                        let univ1 = IUniswapV1Factory::new(factory_address, provider);
                        let pool_addr = match univ1.getExchange(other).call().await {
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
                let v3_fees = vec![100u32, 500u32, 3000u32, 10000u32];
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
            PoolTypeConfig::V4 => {
                // Placeholder
            }
        }
    }

    // Execute batch for pool addresses
    let results = futures::future::join_all(pool_futures).await;

    let mut pool_addresses = Vec::new();
    for (pool_addr, meta) in results {
        if pool_addr != Address::ZERO {
            pool_addresses.push((pool_addr, meta));
        }
    }

    // Now build batch for stablecoin balances
    let mut balance_futures: Vec<
        std::pin::Pin<Box<dyn std::future::Future<Output = (U256, Address, PoolMeta)>>>,
    > = Vec::new();

    for (pool_addr, meta) in &pool_addresses {
        for &stable in &config.stables {
            let pool_addr_clone = *pool_addr;
            let meta_clone = meta.clone();
            let provider = provider.clone();

            balance_futures.push(Box::pin(async move {
                let erc20 = ERC20::new(stable, provider);
                let balance = match erc20.balanceOf(pool_addr_clone).call().await {
                    Ok(r) => r,
                    Err(_) => U256::ZERO,
                };
                (balance, pool_addr_clone, meta_clone)
            })
                as std::pin::Pin<Box<dyn std::future::Future<Output = _>>>);
        }
    }

    // Execute batch for balances
    let balance_results = futures::future::join_all(balance_futures).await;

    let mut valid_pools = Vec::new();

    for (balance, pool_addr, meta) in balance_results {
        // USDC and USDT typically have 6 decimals on ETH, but 18 decimals on BSC.
        let min_balance = if network == "bnb" {
            // 50,000 * 10^18
            U256::from(50_000_000_000_000_000_000_000u128)
        } else {
            // 50,000 * 10^6
            U256::from(50_000_000_000u64)
        };

        if balance > min_balance {
            let config = PoolConfig {
                pair: pool_addr,
                dex: meta.0,
                pool_type: meta.1,
                fee_numerator: None,
                fee_denominator: None,
                fee: meta.2,
            };
            valid_pools.push(config);
        }
    }

    // Deduplicate pools
    valid_pools.sort_by_key(|p| p.pair);
    valid_pools.dedup_by_key(|p| p.pair);

    let output = Output { pools: valid_pools };
    let toml_string = toml::to_string_pretty(&output)?;

    // Создаем output файл вместо вывода в консоль
    let output_filename = format!("{}_output.toml", network);
    fs::write(&output_filename, &toml_string)?;

    println!("Results written to {}", output_filename);
    println!("Found {} pools", output.pools.len());

    Ok(())
}
