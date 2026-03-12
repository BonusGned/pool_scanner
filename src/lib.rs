//! Pool Scanner Library
//!
//! A Rust-based blockchain analytics tool that discovers and validates
//! decentralized exchange (DEX) liquidity pools across multiple EVM-compatible networks.

use alloy::primitives::{Address, U256};
use alloy::sol;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub pools: Vec<PoolConfig>,
}

/// Generate all token pairs to check
/// Returns (token0, token1) combinations
pub fn generate_pairs(stables: &[Address], other_tokens: &[Address]) -> Vec<(Address, Address)> {
    let mut pairs = Vec::new();

    // 1. Stable vs Other
    for &stable in stables {
        for &other in other_tokens {
            pairs.push((stable, other));
        }
    }

    // 2. Stable vs Stable (unique pairs only)
    for i in 0..stables.len() {
        for j in (i + 1)..stables.len() {
            pairs.push((stables[i], stables[j]));
        }
    }

    pairs
}

/// Calculate minimum liquidity threshold based on network
pub fn get_min_liquidity(network: &str) -> U256 {
    if network == "bnb" {
        // 50,000 * 10^18 (18 decimals on BSC)
        U256::from(50_000_000_000_000_000_000_000u128)
    } else {
        // 50,000 * 10^6 (6 decimals on ETH)
        U256::from(50_000_000_000u64)
    }
}

/// Filter pools by liquidity threshold
pub fn filter_pools_by_liquidity(
    _pools: &[(Address, (String, PoolTypeConfig, Option<u32>))],
    balances: &[(U256, Address, (String, PoolTypeConfig, Option<u32>))],
    min_balance: U256,
) -> Vec<PoolConfig> {
    let mut valid_pools = Vec::new();

    for (balance, pool_addr, meta) in balances {
        if *balance > min_balance {
            let config = PoolConfig {
                pair: *pool_addr,
                dex: meta.0.clone(),
                pool_type: meta.1.clone(),
                fee_numerator: None,
                fee_denominator: None,
                fee: meta.2,
            };
            valid_pools.push(config);
        }
    }

    valid_pools
}

/// Deduplicate pools by address
pub fn deduplicate_pools(mut pools: Vec<PoolConfig>) -> Vec<PoolConfig> {
    pools.sort_by_key(|p| p.pair);
    pools.dedup_by_key(|p| p.pair);
    pools
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    mod config_tests {
        use super::*;

        #[test]
        fn test_parse_valid_eth_config() {
            let config_str = r#"
                rpc_url = "https://ethereum-rpc.publicnode.com"
                multicall3_address = "0xcA11bde05977b3631167028862bE2a173976CA11"

                stables = [
                    "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                    "0xdAC17F958D2ee523a2206206994597C13D831ec7",
                ]

                other_tokens = [
                    "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
                ]

                [[factories]]
                name = "UniswapV2"
                address = "0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f"
                type = "v2"

                [[factories]]
                name = "UniswapV3"
                address = "0x1F98431c8aD98523631AE4a59f267346ea31F984"
                type = "v3"
            "#;

            let config: ScannerConfig = toml::from_str(config_str).unwrap();
            assert_eq!(config.rpc_url, "https://ethereum-rpc.publicnode.com");
            assert_eq!(config.stables.len(), 2);
            assert_eq!(config.other_tokens.len(), 1);
            assert_eq!(config.factories.len(), 2);
            assert_eq!(config.factories[0].name, "UniswapV2");
            assert_eq!(config.factories[1].factory_type, PoolTypeConfig::V3);
        }

        #[test]
        fn test_parse_valid_bnb_config() {
            let config_str = r#"
                rpc_url = "https://bsc-dataseed.binance.org/"
                multicall3_address = "0xcA11bde05977b3631167028862bE2a173976CA11"

                stables = [
                    "0x55d398326f99059ff775485246999027b3197955",
                    "0x8ac76a51cc950d9822d68b83fe1ad97b32cd580d",
                ]

                other_tokens = [
                    "0xbb4cdb9cbd36b01bd1cbaebf2de08d9173bc095c",
                ]

                [[factories]]
                name = "PancakeSwapV2"
                address = "0xcA143Ce32Fe78f1f7019d7d551a6402fC5350c73"
                type = "v2"

                [[factories]]
                name = "PancakeSwapV3"
                address = "0x0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865"
                type = "v3"

                [[factories]]
                name = "biswap"
                address = "0x858E3312ed3A876947EA49d572A7C42DE08af7EE"
                type = "v2"
            "#;

            let config: ScannerConfig = toml::from_str(config_str).unwrap();
            assert_eq!(config.factories.len(), 3);
            assert_eq!(config.factories[0].name, "PancakeSwapV2");
            assert_eq!(config.factories[2].name, "biswap");
        }

        #[test]
        fn test_parse_invalid_config() {
            let invalid_config = r#"
                rpc_url = "not-a-valid-url"
                stables = ["invalid-address"]
            "#;

            let result: Result<ScannerConfig, _> = toml::from_str(invalid_config);
            assert!(result.is_err());
        }

        #[test]
        fn test_parse_all_factory_types() {
            let config_str = r#"
                rpc_url = "https://example.com"
                multicall3_address = "0xcA11bde05977b3631167028862bE2a173976CA11"
                stables = []
                other_tokens = []

                [[factories]]
                name = "V1"
                address = "0x0000000000000000000000000000000000000001"
                type = "v1"

                [[factories]]
                name = "V2"
                address = "0x0000000000000000000000000000000000000002"
                type = "v2"

                [[factories]]
                name = "V3"
                address = "0x0000000000000000000000000000000000000003"
                type = "v3"

                [[factories]]
                name = "V4"
                address = "0x0000000000000000000000000000000000000004"
                type = "v4"
            "#;

            let config: ScannerConfig = toml::from_str(config_str).unwrap();
            assert_eq!(config.factories.len(), 4);
            assert_eq!(config.factories[0].factory_type, PoolTypeConfig::V1);
            assert_eq!(config.factories[1].factory_type, PoolTypeConfig::V2);
            assert_eq!(config.factories[2].factory_type, PoolTypeConfig::V3);
            assert_eq!(config.factories[3].factory_type, PoolTypeConfig::V4);
        }

        #[test]
        fn test_pool_config_serialization() {
            let pool = PoolConfig {
                pair: address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640"),
                dex: "UniswapV3".to_string(),
                pool_type: PoolTypeConfig::V3,
                fee_numerator: None,
                fee_denominator: None,
                fee: Some(500),
            };

            let serialized = toml::to_string(&pool).unwrap();
            assert!(serialized.contains("dex = \"UniswapV3\""));
            assert!(serialized.contains("pool_type = \"v3\""));
            assert!(serialized.contains("fee = 500"));
        }

        #[test]
        fn test_output_serialization() {
            let output = Output {
                pools: vec![
                    PoolConfig {
                        pair: address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640"),
                        dex: "UniswapV3".to_string(),
                        pool_type: PoolTypeConfig::V3,
                        fee_numerator: None,
                        fee_denominator: None,
                        fee: Some(500),
                    },
                    PoolConfig {
                        pair: address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
                        dex: "UniswapV2".to_string(),
                        pool_type: PoolTypeConfig::V2,
                        fee_numerator: None,
                        fee_denominator: None,
                        fee: None,
                    },
                ],
            };

            let serialized = toml::to_string_pretty(&output).unwrap();
            assert!(serialized.contains("[[pools]]"));
            assert!(serialized.contains("UniswapV3"));
            assert!(serialized.contains("UniswapV2"));
        }
    }

    mod pair_generation_tests {
        use super::*;

        #[test]
        fn test_generate_pairs_stable_vs_other() {
            let stables = vec![
                address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
                address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"), // USDT
            ];
            let other_tokens = vec![address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")]; // WETH

            let pairs = generate_pairs(&stables, &other_tokens);

            // Should generate 2 pairs (2 stables × 1 other) + 1 stable-stable pair = 3
            assert_eq!(pairs.len(), 3);

            // Verify pairs contain correct combinations
            assert!(pairs.contains(&(stables[0], other_tokens[0])));
            assert!(pairs.contains(&(stables[1], other_tokens[0])));
        }

        #[test]
        fn test_generate_pairs_stable_vs_stable() {
            let stables = vec![
                address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
                address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"), // USDT
                address!("0x6B175474E89094C44Da98b954EedeAC495271d0F"), // DAI
            ];
            let other_tokens = vec![];

            let pairs = generate_pairs(&stables, &other_tokens);

            // Should generate 3 unique pairs (3 choose 2)
            assert_eq!(pairs.len(), 3);

            // Verify all unique combinations exist
            assert!(pairs.contains(&(stables[0], stables[1])));
            assert!(pairs.contains(&(stables[0], stables[2])));
            assert!(pairs.contains(&(stables[1], stables[2])));
        }

        #[test]
        fn test_generate_pairs_combined() {
            let stables = vec![
                address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
                address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
            ];
            let other_tokens = vec![
                address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
                address!("0x7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9"),
            ];

            let pairs = generate_pairs(&stables, &other_tokens);

            // 2 stables × 2 others + 1 stable-stable pair = 5 pairs
            assert_eq!(pairs.len(), 5);
        }

        #[test]
        fn test_generate_pairs_empty_inputs() {
            let pairs = generate_pairs(&[], &[]);
            assert_eq!(pairs.len(), 0);

            let stables = vec![address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")];
            let pairs = generate_pairs(&stables, &[]);
            assert_eq!(pairs.len(), 0);

            let other = vec![address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")];
            let pairs = generate_pairs(&[], &other);
            assert_eq!(pairs.len(), 0);
        }

        #[test]
        fn test_generate_pairs_no_duplicates() {
            let stables = vec![
                address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
                address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
            ];
            let other_tokens = vec![address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")];

            let pairs = generate_pairs(&stables, &other_tokens);

            // Check no duplicate pairs
            let unique_pairs: std::collections::HashSet<_> = pairs.iter().collect();
            assert_eq!(unique_pairs.len(), pairs.len());
        }
    }

    mod liquidity_filter_tests {
        use super::*;

        #[test]
        fn test_get_min_liquidity_eth() {
            let min = get_min_liquidity("eth");
            // 50,000 * 10^6
            assert_eq!(min, U256::from(50_000_000_000u64));
        }

        #[test]
        fn test_get_min_liquidity_bnb() {
            let min = get_min_liquidity("bnb");
            // 50,000 * 10^18
            assert_eq!(min, U256::from(50_000_000_000_000_000_000_000u128));
        }

        #[test]
        fn test_get_min_liquidity_default() {
            // Default should be ETH threshold
            let min = get_min_liquidity("arbitrum");
            assert_eq!(min, U256::from(50_000_000_000u64));
        }

        #[test]
        fn test_filter_pools_above_threshold() {
            let pool_addr = address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");
            let meta = ("UniswapV3".to_string(), PoolTypeConfig::V3, Some(500u32));
            let min_balance = U256::from(50_000_000_000u64);

            let pools = vec![(pool_addr, meta.clone())];
            let balances = vec![(U256::from(100_000_000_000u64), pool_addr, meta)];

            let filtered = filter_pools_by_liquidity(&pools, &balances, min_balance);

            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].pair, pool_addr);
            assert_eq!(filtered[0].dex, "UniswapV3");
        }

        #[test]
        fn test_filter_pools_below_threshold() {
            let pool_addr = address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");
            let meta = ("UniswapV3".to_string(), PoolTypeConfig::V3, Some(500u32));
            let min_balance = U256::from(50_000_000_000u64);

            let pools = vec![(pool_addr, meta.clone())];
            let balances = vec![(U256::from(10_000_000_000u64), pool_addr, meta)];

            let filtered = filter_pools_by_liquidity(&pools, &balances, min_balance);

            assert_eq!(filtered.len(), 0);
        }

        #[test]
        fn test_filter_pools_at_threshold() {
            let pool_addr = address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");
            let meta = ("UniswapV3".to_string(), PoolTypeConfig::V3, Some(500u32));
            let min_balance = U256::from(50_000_000_000u64);

            let pools = vec![(pool_addr, meta.clone())];
            // Exactly at threshold should be excluded (> not >=)
            let balances = vec![(min_balance, pool_addr, meta)];

            let filtered = filter_pools_by_liquidity(&pools, &balances, min_balance);

            assert_eq!(filtered.len(), 0);
        }

        #[test]
        fn test_filter_pools_multiple() {
            let pool1 = address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");
            let pool2 = address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc");
            let pool3 = address!("0x0d4a11d5EEaaC28EC3F61d100daF4d40471f1852");

            let meta1 = ("UniswapV3".to_string(), PoolTypeConfig::V3, Some(500u32));
            let meta2 = ("UniswapV2".to_string(), PoolTypeConfig::V2, None);
            let meta3 = ("PancakeSwap".to_string(), PoolTypeConfig::V2, None);

            let min_balance = U256::from(50_000_000_000u64);

            let pools = vec![
                (pool1, meta1.clone()),
                (pool2, meta2.clone()),
                (pool3, meta3.clone()),
            ];
            let balances = vec![
                (U256::from(100_000_000_000u64), pool1, meta1), // Above
                (U256::from(10_000_000_000u64), pool2, meta2),  // Below
                (U256::from(75_000_000_000u64), pool3, meta3),  // Above
            ];

            let filtered = filter_pools_by_liquidity(&pools, &balances, min_balance);

            assert_eq!(filtered.len(), 2);
            assert!(filtered.iter().any(|p| p.pair == pool1));
            assert!(filtered.iter().any(|p| p.pair == pool3));
            assert!(!filtered.iter().any(|p| p.pair == pool2));
        }
    }

    mod deduplication_tests {
        use super::*;

        #[test]
        fn test_deduplicate_pools() {
            let pool_addr = address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");

            let pools = vec![
                PoolConfig {
                    pair: pool_addr,
                    dex: "UniswapV3".to_string(),
                    pool_type: PoolTypeConfig::V3,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: Some(500),
                },
                PoolConfig {
                    pair: pool_addr,
                    dex: "UniswapV3".to_string(),
                    pool_type: PoolTypeConfig::V3,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: Some(500),
                },
            ];

            let deduplicated = deduplicate_pools(pools);
            assert_eq!(deduplicated.len(), 1);
        }

        #[test]
        fn test_deduplicate_pools_different_addresses() {
            let pools = vec![
                PoolConfig {
                    pair: address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640"),
                    dex: "UniswapV3".to_string(),
                    pool_type: PoolTypeConfig::V3,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: Some(500),
                },
                PoolConfig {
                    pair: address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
                    dex: "UniswapV2".to_string(),
                    pool_type: PoolTypeConfig::V2,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: None,
                },
            ];

            let deduplicated = deduplicate_pools(pools);
            assert_eq!(deduplicated.len(), 2);
        }

        #[test]
        fn test_deduplicate_pools_empty() {
            let pools: Vec<PoolConfig> = vec![];
            let deduplicated = deduplicate_pools(pools);
            assert_eq!(deduplicated.len(), 0);
        }

        #[test]
        fn test_deduplicate_pools_sorted() {
            let pools = vec![
                PoolConfig {
                    pair: address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
                    dex: "UniswapV2".to_string(),
                    pool_type: PoolTypeConfig::V2,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: None,
                },
                PoolConfig {
                    pair: address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640"),
                    dex: "UniswapV3".to_string(),
                    pool_type: PoolTypeConfig::V3,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: Some(500),
                },
            ];

            let deduplicated = deduplicate_pools(pools);

            // Should be sorting by address
            assert!(deduplicated[0].pair < deduplicated[1].pair);
        }

        #[test]
        fn test_deduplicate_pools_multiple_duplicates() {
            let addr1 = address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");
            let addr2 = address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc");

            let pools = vec![
                PoolConfig {
                    pair: addr1,
                    dex: "DEX1".to_string(),
                    pool_type: PoolTypeConfig::V3,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: Some(500),
                },
                PoolConfig {
                    pair: addr2,
                    dex: "DEX2".to_string(),
                    pool_type: PoolTypeConfig::V2,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: None,
                },
                PoolConfig {
                    pair: addr1,
                    dex: "DEX3".to_string(),
                    pool_type: PoolTypeConfig::V3,
                    fee_numerator: None,
                    fee_denominator: None,
                    fee: Some(3000),
                },
            ];

            let deduplicated = deduplicate_pools(pools);
            assert_eq!(deduplicated.len(), 2);
        }
    }
}
