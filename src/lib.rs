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
pub struct TokenConfig {
    pub address: Address,
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_liquidity: Option<String>,
}

impl TokenConfig {
    pub fn min_liquidity_u256(&self) -> Option<U256> {
        self.min_liquidity
            .as_ref()
            .map(|s| s.parse::<U256>().unwrap_or_else(|_| panic!("Invalid min_liquidity for {}: {}", self.symbol, s)))
    }
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
    pub tokens: Vec<TokenConfig>,
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

pub fn generate_pairs(tokens: &[TokenConfig]) -> Vec<(Address, Address)> {
    let mut pairs = Vec::new();
    for i in 0..tokens.len() {
        for j in (i + 1)..tokens.len() {
            pairs.push((tokens[i].address, tokens[j].address));
        }
    }
    pairs
}

pub fn liquidity_tokens(tokens: &[TokenConfig]) -> Vec<(Address, U256)> {
    tokens
        .iter()
        .filter_map(|t| t.min_liquidity_u256().map(|liq| (t.address, liq)))
        .collect()
}

pub fn deduplicate_pools(mut pools: Vec<PoolConfig>) -> Vec<PoolConfig> {
    pools.sort_by_key(|p| p.pair);
    pools.dedup_by_key(|p| p.pair);
    pools
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    fn token(addr: Address, symbol: &str, min_liq: Option<&str>) -> TokenConfig {
        TokenConfig {
            address: addr,
            symbol: symbol.to_string(),
            min_liquidity: min_liq.map(String::from),
        }
    }

    mod config_tests {
        use super::*;

        #[test]
        fn test_parse_valid_config() {
            let config_str = r#"
                rpc_url = "https://ethereum-rpc.publicnode.com"
                multicall3_address = "0xcA11bde05977b3631167028862bE2a173976CA11"

                [[tokens]]
                address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
                symbol = "USDC"
                min_liquidity = "50000000000"

                [[tokens]]
                address = "0xdAC17F958D2ee523a2206206994597C13D831ec7"
                symbol = "USDT"
                min_liquidity = "50000000000"

                [[tokens]]
                address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
                symbol = "WETH"

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
            assert_eq!(config.tokens.len(), 3);
            assert_eq!(config.factories.len(), 2);
            assert_eq!(config.factories[0].name, "UniswapV2");
            assert_eq!(config.factories[1].factory_type, PoolTypeConfig::V3);

            let liq = liquidity_tokens(&config.tokens);
            assert_eq!(liq.len(), 2);
        }

        #[test]
        fn test_parse_invalid_config() {
            let invalid_config = r#"
                rpc_url = "not-a-valid-url"
                tokens = ["invalid"]
            "#;

            let result: Result<ScannerConfig, _> = toml::from_str(invalid_config);
            assert!(result.is_err());
        }

        #[test]
        fn test_parse_all_factory_types() {
            let config_str = r#"
                rpc_url = "https://example.com"
                multicall3_address = "0xcA11bde05977b3631167028862bE2a173976CA11"
                tokens = []

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

        #[test]
        fn test_token_config_min_liquidity_parsing() {
            let t = token(Address::ZERO, "USDT", Some("50000000000000000000000"));
            assert_eq!(
                t.min_liquidity_u256().unwrap(),
                U256::from(50_000_000_000_000_000_000_000u128)
            );

            let t_none = token(Address::ZERO, "WBNB", None);
            assert!(t_none.min_liquidity_u256().is_none());
        }
    }

    mod pair_generation_tests {
        use super::*;

        #[test]
        fn test_generate_pairs_all_combinations() {
            let tokens = vec![
                token(address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), "USDC", Some("50000000000")),
                token(address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"), "USDT", Some("50000000000")),
                token(address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), "WETH", None),
            ];

            let pairs = generate_pairs(&tokens);

            // 3 choose 2 = 3 unique pairs
            assert_eq!(pairs.len(), 3);
            assert!(pairs.contains(&(tokens[0].address, tokens[1].address)));
            assert!(pairs.contains(&(tokens[0].address, tokens[2].address)));
            assert!(pairs.contains(&(tokens[1].address, tokens[2].address)));
        }

        #[test]
        fn test_generate_pairs_four_tokens() {
            let tokens = vec![
                token(address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), "USDC", None),
                token(address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"), "USDT", None),
                token(address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), "WETH", None),
                token(address!("0x7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9"), "AAVE", None),
            ];

            let pairs = generate_pairs(&tokens);

            // 4 choose 2 = 6 unique pairs
            assert_eq!(pairs.len(), 6);
        }

        #[test]
        fn test_generate_pairs_empty() {
            let pairs = generate_pairs(&[]);
            assert_eq!(pairs.len(), 0);
        }

        #[test]
        fn test_generate_pairs_single_token() {
            let tokens = vec![
                token(address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), "USDC", None),
            ];
            let pairs = generate_pairs(&tokens);
            assert_eq!(pairs.len(), 0);
        }

        #[test]
        fn test_generate_pairs_no_duplicates() {
            let tokens = vec![
                token(address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), "USDC", None),
                token(address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"), "USDT", None),
                token(address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), "WETH", None),
            ];

            let pairs = generate_pairs(&tokens);
            let unique_pairs: std::collections::HashSet<_> = pairs.iter().collect();
            assert_eq!(unique_pairs.len(), pairs.len());
        }
    }

    mod liquidity_tokens_tests {
        use super::*;

        #[test]
        fn test_liquidity_tokens_filters_correctly() {
            let tokens = vec![
                token(address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), "USDC", Some("50000000000")),
                token(address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), "WETH", Some("20000000000000000000")),
                token(address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"), "WBNB", None),
            ];

            let liq = liquidity_tokens(&tokens);
            assert_eq!(liq.len(), 2);
            assert_eq!(liq[0].0, tokens[0].address);
            assert_eq!(liq[0].1, U256::from(50_000_000_000u64));
            assert_eq!(liq[1].0, tokens[1].address);
            assert_eq!(liq[1].1, U256::from(20_000_000_000_000_000_000u128));
        }

        #[test]
        fn test_liquidity_tokens_none() {
            let tokens = vec![
                token(Address::ZERO, "A", None),
                token(Address::ZERO, "B", None),
            ];
            assert!(liquidity_tokens(&tokens).is_empty());
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
