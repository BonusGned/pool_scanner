//! Integration tests for Pool Scanner
//!
//! These tests verify the end-to-end functionality of the pool scanner,
//! including configuration loading, pair generation, and output formatting.

use alloy::primitives::{address, Address, U256};
use pool_scanner::{
    deduplicate_pools, filter_pools_by_liquidity, generate_pairs, get_min_liquidity, FactoryConfig,
    Output, PoolConfig, PoolTypeConfig, ScannerConfig,
};

/// Test loading and parsing the Ethereum configuration file
#[test]
fn test_load_eth_config_from_file() {
    let config_path = "config/eth.toml";
    let config_content =
        std::fs::read_to_string(config_path).expect("Failed to read eth.toml config file");

    let config: ScannerConfig =
        toml::from_str(&config_content).expect("Failed to parse eth.toml as ScannerConfig");

    // Verify basic structure
    assert!(!config.rpc_url.is_empty());
    assert!(config.rpc_url.starts_with("http"));
    assert_ne!(config.multicall3_address, Address::ZERO);

    // Should have stables configured
    assert!(!config.stables.is_empty());
    assert!(config.stables.len() >= 2);

    // Should have other tokens configured
    assert!(!config.other_tokens.is_empty());

    // Should have factories configured
    assert!(!config.factories.is_empty());
    assert!(config
        .factories
        .iter()
        .any(|f| f.factory_type == PoolTypeConfig::V2));
    assert!(config
        .factories
        .iter()
        .any(|f| f.factory_type == PoolTypeConfig::V3));
}

/// Test loading and parsing the BNB configuration file
#[test]
fn test_load_bnb_config_from_file() {
    let config_path = "config/bnb.toml";
    let config_content =
        std::fs::read_to_string(config_path).expect("Failed to read bnb.toml config file");

    let config: ScannerConfig =
        toml::from_str(&config_content).expect("Failed to parse bnb.toml as ScannerConfig");

    // Verify basic structure
    assert!(!config.rpc_url.is_empty());
    assert!(config.rpc_url.starts_with("http"));
    assert_ne!(config.multicall3_address, Address::ZERO);

    // Should have stables configured
    assert!(!config.stables.is_empty());
    assert!(config.stables.len() >= 2);

    // Should have other tokens configured
    assert!(!config.other_tokens.is_empty());

    // Should have multiple factories (PancakeSwap, Biswap, etc.)
    assert!(config.factories.len() >= 3);
}

/// Test that configuration files are valid TOML
#[test]
fn test_config_files_valid_toml() {
    let config_files = ["config/eth.toml", "config/bnb.toml"];

    for config_path in &config_files {
        let content = std::fs::read_to_string(config_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", config_path));

        // Should parse as generic TOML
        let _: toml::Value = toml::from_str(&content)
            .unwrap_or_else(|_| panic!("{} is not valid TOML", config_path));
    }
}

/// Test end-to-end pool processing workflow
#[test]
fn test_end_to_end_pool_processing() {
    // Simulate a complete workflow with mock data
    let stables = vec![
        address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"), // USDC
        address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"), // USDT
    ];
    let other_tokens = vec![address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")]; // WETH

    // Step 1: Generate pairs
    let pairs = generate_pairs(&stables, &other_tokens);
    assert_eq!(pairs.len(), 3); // 2 stables × 1 other + 1 stable-stable

    // Step 2: Simulate discovered pools
    let pool1 = address!("0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640");
    let pool2 = address!("0xB4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc");
    let pool3 = address!("0x0d4a11d5EEaaC28EC3F61d100daF4d40471f1852");

    let meta1 = ("UniswapV3".to_string(), PoolTypeConfig::V3, Some(500u32));
    let meta2 = ("UniswapV2".to_string(), PoolTypeConfig::V2, None);
    let meta3 = ("UniswapV2".to_string(), PoolTypeConfig::V2, None);

    let pools = vec![
        (pool1, meta1.clone()),
        (pool2, meta2.clone()),
        (pool3, meta3.clone()),
    ];

    // Step 3: Simulate balance checks
    let balances = vec![
        (U256::from(100_000_000_000u64), pool1, meta1.clone()), // Above threshold
        (U256::from(10_000_000_000u64), pool2, meta2.clone()),  // Below threshold
        (U256::from(75_000_000_000u64), pool3, meta3.clone()),  // Above threshold
    ];

    // Step 4: Filter by liquidity
    let min_balance = get_min_liquidity("eth");
    let filtered = filter_pools_by_liquidity(&pools, &balances, min_balance);
    assert_eq!(filtered.len(), 2);

    // Step 5: Deduplicate
    let final_pools = deduplicate_pools(filtered);
    assert_eq!(final_pools.len(), 2);

    // Step 6: Create output
    let output = Output { pools: final_pools };

    // Step 7: Serialize to TOML
    let toml_string = toml::to_string_pretty(&output).expect("Failed to serialize output");

    // Verify output format
    assert!(toml_string.contains("[[pools]]"));
    assert!(toml_string.contains("UniswapV3"));
    assert!(toml_string.contains("UniswapV2"));
    assert!(toml_string.contains("pool_type = \"v3\""));
    assert!(toml_string.contains("pool_type = \"v2\""));
}

/// Test network-specific liquidity thresholds
#[test]
fn test_network_liquidity_thresholds() {
    // Ethereum threshold (6 decimals)
    let eth_threshold = get_min_liquidity("eth");
    assert_eq!(eth_threshold, U256::from(50_000_000_000u64));

    // BNB threshold (18 decimals)
    let bnb_threshold = get_min_liquidity("bnb");
    assert_eq!(
        bnb_threshold,
        U256::from(50_000_000_000_000_000_000_000u128)
    );

    // BNB threshold should be larger (more wei due to 18 decimals)
    assert!(bnb_threshold > eth_threshold);

    // Default (non-bnb) should use ETH threshold
    let default_threshold = get_min_liquidity("arbitrum");
    assert_eq!(default_threshold, eth_threshold);
}

/// Test pair generation with real config data
#[test]
fn test_pair_generation_with_real_config() {
    let config_path = "config/eth.toml";
    let config_content = std::fs::read_to_string(config_path).unwrap();
    let config: ScannerConfig = toml::from_str(&config_content).unwrap();

    let pairs = generate_pairs(&config.stables, &config.other_tokens);

    // Should generate at least some pairs
    assert!(!pairs.is_empty());

    // All pairs should contain valid addresses
    for (token0, token1) in &pairs {
        assert_ne!(*token0, Address::ZERO);
        assert_ne!(*token1, Address::ZERO);
        assert_ne!(token0, token1); // No self-pairs
    }
}

/// Test output file format
#[test]
fn test_output_file_format() {
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

    let output = Output { pools };
    let toml_string = toml::to_string_pretty(&output).unwrap();

    // Parse back to verify round-trip
    let parsed: Output = toml::from_str(&toml_string).unwrap();
    assert_eq!(parsed.pools.len(), 2);

    // Verify fields are preserved
    assert_eq!(parsed.pools[0].dex, "UniswapV3");
    assert_eq!(parsed.pools[0].fee, Some(500));
    assert_eq!(parsed.pools[1].dex, "UniswapV2");
    assert_eq!(parsed.pools[1].fee, None);
}

/// Test factory configuration variants
#[test]
fn test_factory_config_variants() {
    // Test all factory types are properly configured
    let factories = vec![
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

/// Test large-scale pool processing
#[test]
fn test_large_scale_pool_processing() {
    // Simulate processing many pools
    let mut pools = Vec::new();
    let mut balances = Vec::new();

    for i in 0..100 {
        let pool_addr = Address::from([i as u8; 20]);
        let meta = (format!("DEX{}", i), PoolTypeConfig::V2, None);
        pools.push((pool_addr, meta.clone()));

        // Half above threshold, half below
        let balance = if i % 2 == 0 {
            U256::from(100_000_000_000u64)
        } else {
            U256::from(10_000_000_000u64)
        };
        balances.push((balance, pool_addr, meta));
    }

    let min_balance = get_min_liquidity("eth");
    let filtered = filter_pools_by_liquidity(&pools, &balances, min_balance);

    // Should have 50 pools (every even index)
    assert_eq!(filtered.len(), 50);

    let deduplicated = deduplicate_pools(filtered);
    assert_eq!(deduplicated.len(), 50);
}
