use alloy::primitives::{address, Address, U256};
use pool_scanner::{
    deduplicate_pools, generate_pairs, liquidity_tokens, FactoryConfig, Output, PoolConfig,
    PoolTypeConfig, ScannerConfig, TokenConfig,
};

fn token(addr: Address, symbol: &str, min_liq: Option<&str>, decimals: Option<u8>) -> TokenConfig {
    TokenConfig {
        address: addr,
        symbol: symbol.to_string(),
        decimals,
        min_liquidity: min_liq.map(String::from),
    }
}

#[test]
fn test_load_eth_config_from_file() {
    let config_content =
        std::fs::read_to_string("config/eth.toml").expect("Failed to read eth.toml");
    let config: ScannerConfig = toml::from_str(&config_content).expect("Failed to parse eth.toml");

    assert!(!config.rpc_url.is_empty());
    assert!(config.rpc_url.starts_with("http"));
    assert_ne!(config.multicall3_address, Address::ZERO);
    assert!(config.tokens.len() >= 3);
    assert!(!config.factories.is_empty());
    assert!(config
        .factories
        .iter()
        .any(|f| f.factory_type == PoolTypeConfig::V2));
    assert!(config
        .factories
        .iter()
        .any(|f| f.factory_type == PoolTypeConfig::V3));

    let liq = liquidity_tokens(&config.tokens);
    assert!(liq.len() >= 2);
}

#[test]
fn test_load_bnb_config_from_file() {
    let config_content =
        std::fs::read_to_string("config/bnb.toml").expect("Failed to read bnb.toml");
    let config: ScannerConfig = toml::from_str(&config_content).expect("Failed to parse bnb.toml");

    assert!(!config.rpc_url.is_empty());
    assert!(config.tokens.len() >= 3);
    assert!(config.factories.len() >= 3);

    let liq = liquidity_tokens(&config.tokens);
    assert!(liq.len() >= 2);
    for (_, min_liq) in &liq {
        assert!(*min_liq > U256::ZERO);
    }
}

#[test]
fn test_config_files_valid_toml() {
    for config_path in &["config/eth.toml", "config/bnb.toml"] {
        let content = std::fs::read_to_string(config_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", config_path));
        let _: toml::Value = toml::from_str(&content)
            .unwrap_or_else(|_| panic!("{} is not valid TOML", config_path));
    }
}

#[test]
fn test_end_to_end_pool_processing() {
    let tokens = vec![
        token(
            address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            "USDC",
            Some("50000"),
            Some(6),
        ),
        token(
            address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
            "USDT",
            Some("50000"),
            Some(6),
        ),
        token(
            address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
            "WETH",
            Some("20"),
            Some(18),
        ),
    ];

    let pairs = generate_pairs(&tokens);
    assert_eq!(pairs.len(), 3);

    let liq = liquidity_tokens(&tokens);
    assert_eq!(liq.len(), 3);

    let pool1 = address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");
    let pool2 = address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc");
    let pool3 = address!("0x0d4a11d5EEaaC28EC3F61d100daF4d40471f1852");

    // Simulate: pool1 has 100k USDC (above 50k threshold)
    // pool2 has 10k USDT (below 50k threshold)
    // pool3 has 25 WETH (above 20 WETH threshold)
    type BalanceResult = (U256, U256, Address, (String, PoolTypeConfig, Option<u32>));
    let balance_results: Vec<BalanceResult> = vec![
        (
            U256::from(100_000_000_000u64),
            U256::from(50_000_000_000u64),
            pool1,
            ("UniswapV3".to_string(), PoolTypeConfig::V3, Some(500)),
        ),
        (
            U256::from(10_000_000_000u64),
            U256::from(50_000_000_000u64),
            pool2,
            ("UniswapV2".to_string(), PoolTypeConfig::V2, None),
        ),
        (
            U256::from(25_000_000_000_000_000_000u128),
            U256::from(20_000_000_000_000_000_000u128),
            pool3,
            ("UniswapV2".to_string(), PoolTypeConfig::V2, None),
        ),
    ];

    let mut valid_pools = Vec::new();
    for (balance, min_liq, pool_addr, meta) in &balance_results {
        if balance > min_liq {
            valid_pools.push(PoolConfig {
                pair: *pool_addr,
                dex: meta.0.clone(),
                pool_type: meta.1.clone(),
                token0: None,
                token1: None,
                fee_numerator: None,
                fee_denominator: None,
                fee: meta.2,
            });
        }
    }
    assert_eq!(valid_pools.len(), 2);

    let final_pools = deduplicate_pools(valid_pools);
    assert_eq!(final_pools.len(), 2);

    let output = Output { pools: final_pools };
    let toml_string = toml::to_string_pretty(&output).expect("Failed to serialize output");
    assert!(toml_string.contains("[[pools]]"));
    assert!(toml_string.contains("UniswapV3"));
    assert!(toml_string.contains("UniswapV2"));
}

#[test]
fn test_per_token_liquidity_thresholds() {
    let tokens = vec![
        token(
            address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
            "USDC",
            Some("50000"),
            Some(6),
        ),
        token(
            address!("0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"),
            "WBTC",
            Some("1"), // 1 BTC (8 decimals)
            Some(8),
        ),
        token(
            address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
            "WETH",
            Some("20"), // 20 ETH (18 decimals)
            Some(18),
        ),
        token(
            address!("0xbb4cdb9cbd36b01bd1cbaebf2de08d9173bc095c"),
            "wBNB",
            None,
            None,
        ),
    ];

    let liq = liquidity_tokens(&tokens);
    assert_eq!(liq.len(), 3);
    assert_eq!(liq[0].1, U256::from(50_000_000_000u64));
    assert_eq!(liq[1].1, U256::from(100_000_000u64));
    assert_eq!(liq[2].1, U256::from(20_000_000_000_000_000_000u128));

    let pairs = generate_pairs(&tokens);
    // 4 choose 2 = 6
    assert_eq!(pairs.len(), 6);
}

#[test]
fn test_pair_generation_with_real_config() {
    let config_content = std::fs::read_to_string("config/eth.toml").unwrap();
    let config: ScannerConfig = toml::from_str(&config_content).unwrap();

    let pairs = generate_pairs(&config.tokens);
    assert!(!pairs.is_empty());

    for (token0, token1) in &pairs {
        assert_ne!(*token0, Address::ZERO);
        assert_ne!(*token1, Address::ZERO);
        assert_ne!(token0, token1);
    }
}

#[test]
fn test_output_file_format() {
    let pools = vec![
        PoolConfig {
            pair: address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640"),
            dex: "UniswapV3".to_string(),
            pool_type: PoolTypeConfig::V3,
            token0: None,
            token1: None,
            fee_numerator: None,
            fee_denominator: None,
            fee: Some(500),
        },
        PoolConfig {
            pair: address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"),
            dex: "UniswapV2".to_string(),
            pool_type: PoolTypeConfig::V2,
            token0: None,
            token1: None,
            fee_numerator: None,
            fee_denominator: None,
            fee: None,
        },
    ];

    let output = Output { pools };
    let toml_string = toml::to_string_pretty(&output).unwrap();

    let parsed: Output = toml::from_str(&toml_string).unwrap();
    assert_eq!(parsed.pools.len(), 2);
    assert_eq!(parsed.pools[0].dex, "UniswapV3");
    assert_eq!(parsed.pools[0].fee, Some(500));
    assert_eq!(parsed.pools[1].dex, "UniswapV2");
    assert_eq!(parsed.pools[1].fee, None);
}

#[test]
fn test_factory_config_variants() {
    let factories = [
        FactoryConfig {
            name: "TestV1".to_string(),
            address: address!("0x0000000000000000000000000000000000000001"),
            factory_type: PoolTypeConfig::V1,
        },
        FactoryConfig {
            name: "TestV2".to_string(),
            address: address!("0x0000000000000000000000000000000000000002"),
            factory_type: PoolTypeConfig::V2,
        },
        FactoryConfig {
            name: "TestV3".to_string(),
            address: address!("0x0000000000000000000000000000000000000003"),
            factory_type: PoolTypeConfig::V3,
        },
        FactoryConfig {
            name: "TestV4".to_string(),
            address: address!("0x0000000000000000000000000000000000000004"),
            factory_type: PoolTypeConfig::V4,
        },
    ];

    assert_eq!(factories.len(), 4);
    assert_eq!(factories[0].factory_type, PoolTypeConfig::V1);
    assert_eq!(factories[1].factory_type, PoolTypeConfig::V2);
    assert_eq!(factories[2].factory_type, PoolTypeConfig::V3);
    assert_eq!(factories[3].factory_type, PoolTypeConfig::V4);
}

#[test]
fn test_large_scale_pool_processing() {
    let mut pools = Vec::new();
    let min_liq = U256::from(50_000_000_000u64);

    for i in 0u8..100 {
        let pool_addr = Address::from([i; 20]);
        let balance = if i % 2 == 0 {
            U256::from(100_000_000_000u64)
        } else {
            U256::from(10_000_000_000u64)
        };

        if balance > min_liq {
            pools.push(PoolConfig {
                pair: pool_addr,
                dex: format!("DEX{}", i),
                pool_type: PoolTypeConfig::V2,
                token0: None,
                token1: None,
                fee_numerator: None,
                fee_denominator: None,
                fee: None,
            });
        }
    }

    assert_eq!(pools.len(), 50);
    let deduplicated = deduplicate_pools(pools);
    assert_eq!(deduplicated.len(), 50);
}
