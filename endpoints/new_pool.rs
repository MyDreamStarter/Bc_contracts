use crate::consts::{
    BP_FEE_KEY, DEFAULT_MAX_M, DEFAULT_MAX_M_LP, DEFAULT_PRICE_FACTOR_DENOMINATOR,
    DEFAULT_PRICE_FACTOR_NUMERATOR, MAX_AIRDROPPED_TOKENS, MAX_LINEAR, MAX_MEME_TOKENS, MIN_LINEAR,
};
use crate::err;
use crate::err::AmmError;
use crate::models::bound::{compute_alpha_abs, compute_beta, BoundPool, Config, Decimals};
use crate::models::fees::FEE;
use crate::models::fees::{Fees, MEME_FEE};
use crate::models::target_config::TargetConfig;
use crate::models::Reserve;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::token::{self, Mint, Token, TokenAccount};
/// Represents the accounts required for creating a new pool.
///
/// This struct defines the accounts needed for the `new_pool` instruction. It includes the sender's account,
/// the pool account, meme mint and vault accounts, quote mint and vault accounts, fee quote vault account,
/// target configuration account, pool signer account, and the system and token programs.
///
/// The `NewPool` struct is used to validate and manage the creation of a new pool within the AMM system.
/// It ensures that all necessary accounts are present and meet the required conditions for pool creation.
#[derive(Accounts)]
pub struct NewPool<'info> {
    #[account(mut)]
    /// The account of the sender initiating the pool creation.
    pub sender: Signer<'info>,
    #[account(
        init,
        payer = sender,
        space = BoundPool::space(),
        seeds = [BoundPool::POOL_PREFIX, meme_mint.key().as_ref(), quote_mint.key().as_ref()],
        bump
    )]
    /// The account representing the pool being created.
    pub pool: Account<'info, BoundPool>,
    #[account(
        mut,
        constraint = meme_mint.mint_authority == COption::Some(pool_signer.key())
            @ err::acc("Meme mint authority must be the pool signer"),
        constraint = meme_mint.freeze_authority == COption::None
            @ err::acc("Meme mint mustn't have a freeze authority"),
    )]
    /// The account representing the meme mint.
    pub meme_mint: Account<'info, Mint>,
    #[account(
        constraint = quote_vault.mint == quote_mint.key()
            @ err::acc("Quote vault must be of ticket mint"),
        constraint = quote_vault.owner == pool_signer.key()
            @ err::acc("Quote vault authority must match the pool signer"),
        constraint = quote_vault.close_authority == COption::None
            @ err::acc("Quote vault must not have close authority"),
        constraint = quote_vault.delegate == COption::None
            @ err::acc("Quote vault must not have delegate"),
    )]
    /// The account representing the quote vault.
    pub quote_vault: Account<'info, TokenAccount>,
    /// The account representing the quote mint.
    pub quote_mint: Account<'info, Mint>,
    #[account(
        constraint = fee_quote_vault.mint == quote_mint.key()
            @ err::acc("Fee quote vault must be of quote mint"),
        constraint = fee_quote_vault.owner == BP_FEE_KEY
            @ err::acc("Fee quote vault authority must match fee key"),
        constraint = fee_quote_vault.close_authority == COption::None
            @ err::acc("Fee quote vault must not have close authority"),
        constraint = fee_quote_vault.delegate == COption::None
            @ err::acc("Fee quote vault must not have delegate"),
    )]
    /// The account representing the fee quote vault.
    pub fee_quote_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = meme_vault.mint == meme_mint.key()
            @ err::acc("Meme vault must be of meme mint"),
        constraint = meme_vault.owner == pool_signer.key()
            @ err::acc("Meme vault authority must match the pool signer"),
        constraint = meme_vault.close_authority == COption::None
            @ err::acc("Meme vault must not have close authority"),
        constraint = meme_vault.delegate == COption::None
            @ err::acc("Meme vault must not have delegate"),
    )]
    /// The account representing the meme vault.
    pub meme_vault: Account<'info, TokenAccount>,
    #[account(
        constraint = target_config.token_mint == quote_mint.key()
    )]
    /// The account representing the target configuration.
    pub target_config: Account<'info, TargetConfig>,
    /// CHECK: pool_pda
    #[account(seeds = [BoundPool::SIGNER_PDA_PREFIX, pool.key().as_ref()], bump)]
    /// The account representing the pool signer.
    pub pool_signer: AccountInfo<'info>,
    /// The system program account.
    pub system_program: Program<'info, System>,
    /// The token program account.
    pub token_program: Program<'info, Token>,
}

impl<'info> NewPool<'info> {
    /// Creates a CPI context for minting meme tokens to the meme vault.
    ///
    /// This function prepares the necessary accounts and program for minting meme tokens to the meme vault.
    /// It ensures that the minting operation is performed by the pool signer and that the tokens are minted to the correct vault.
    ///
    /// # Returns
    ///
    /// A CPI context for minting meme tokens to the meme vault.
    fn mint_meme_tokens(&self) -> CpiContext<'_, '_, '_, 'info, token::MintTo<'info>> {
        let cpi_accounts = token::MintTo {
            mint: self.meme_mint.to_account_info(),
            to: self.meme_vault.to_account_info(),
            authority: self.pool_signer.to_account_info(),
        };

        let cpi_program = self.token_program.to_account_info();
        CpiContext::new(cpi_program, cpi_accounts)
    }
}

/// Handles the creation of a new pool.
///
/// This function initializes a new pool by minting meme tokens, setting up the pool's configuration,
/// and updating the pool's state. It ensures that the meme mint authority is the pool signer, and
/// that the meme mint does not have a freeze authority. It also verifies that the quote vault is of
/// the correct mint, owned by the pool signer, and does not have close or delegate authorities.
/// Additionally, it checks the fee quote vault's mint, ownership, and authorities.
///
/// # Parameters
///
/// * `ctx`: The context of the current instruction.
/// * `airdropped_tokens`: The number of tokens to be airdropped.
/// * `vesting_period`: The duration of the vesting period in seconds.
///
/// # Returns
///
/// A result indicating the success or failure of the operation.
///
/// Sam's Journey: Creating a New Token Pool
///
/// Sam wants to:
/// 1. Create 1B total tokens
/// 2. Set aside up to 100M for airdrops
/// 3. Set a 7-day vesting period
/// 4. Configure automated trading
///
/// # Parameters
/// * `ctx` - The context containing all necessary accounts
/// * `airdropped_tokens` - Amount of tokens for airdrop (max 100M)
/// * `vesting_period` - Time tokens are locked (in seconds)
pub fn handle(ctx: Context<NewPool>, airdropped_tokens: u64, vesting_period: i64) -> Result<()> {
    let accs = ctx.accounts;

    // Chapter 1: Initial Checks 🔍
    // Ensure we're starting with a fresh token mint
    if accs.meme_mint.supply != 0 {
        return Err(error!(AmmError::NonZeroInitialMemeSupply));
    }

    // Ensure we're not airdropping too many tokens
    if airdropped_tokens > MAX_AIRDROPPED_TOKENS {
        return Err(error!(AmmError::AirdroppedTokensOvercap));
    }

    // Ensure the vesting period is within the allowed range
    // Check Sam's vesting period (between 1-13 days)
    if MIN_LINEAR > vesting_period || vesting_period > MAX_LINEAR {
        return Err(error!(AmmError::InvalidVestingPeriod));
    }

    // Chapter 2: Minting Meme Tokens 💰
    // Prepare the seeds for the pool signer PDA
    let seeds = &[
        BoundPool::SIGNER_PDA_PREFIX,    // "pool_signer"
        &accs.pool.key().to_bytes()[..], // Pool's address
        &[ctx.bumps.pool_signer],        // Unique bump seed
    ];

    let signer_seeds = &[&seeds[..]];

    // Mint all 1B tokens to pool vault
    token::mint_to(
        accs.mint_meme_tokens().with_signer(signer_seeds),
        MAX_MEME_TOKENS as u64, // 1B total tokens
    )
    .unwrap();

    // Chapter 3: Configuring Pool Settings ⚙️
    let pool = &mut accs.pool;

    // Set up fee collection vault
    pool.fee_vault_quote = accs.fee_quote_vault.key();

    // Initialize SOL reserve
    pool.quote_reserve = Reserve {
        tokens: 0,// Start with 0 SOL
        mint: accs.quote_mint.key(),// SOL mint address
        vault: accs.quote_vault.key(),// SOL vault address
    };

    // Configure trading fees
    pool.fees = Fees {
        fee_meme_percent: MEME_FEE,
        fee_quote_percent: FEE,
    };

   // Chapter 4: Setting Up Price Mathematics 📊
    // Calculate SOL decimal precision (1B = 1 SOL)
    let mint_decimals = 10_u128
        .checked_pow(accs.quote_mint.decimals as u32)
        .unwrap();
    
    // Configure bonding curve parameters
    let gamma_s = accs.target_config.token_target_amount as u128;  // SOL target
    let gamma_m = DEFAULT_MAX_M;        // 690M trading tokens
    let omega_m = DEFAULT_MAX_M_LP;     // 310M LP tokens
    let price_factor_num = DEFAULT_PRICE_FACTOR_NUMERATOR;     // Price adjustment
    let price_factor_denom = DEFAULT_PRICE_FACTOR_DENOMINATOR; // factors

    // Calculate price curve slope (α)
    let (alpha_abs, decimals) = compute_alpha_abs(
        gamma_s, mint_decimals, gamma_m, omega_m,
        price_factor_num, price_factor_denom,
    )?;

    // Chapter 5: Finalizing Pool Configuration 🎯
    pool.config = Config {
        alpha_abs,           // Price curve slope (α)
        beta: compute_beta(  // Starting price (β)
            gamma_s, mint_decimals, gamma_m, omega_m,
            price_factor_num, price_factor_denom, decimals,
        )?,
        gamma_s: gamma_s as u64,    // SOL target amount
        gamma_m: gamma_m as u64,    // Trading token amount
        omega_m: omega_m as u64,    // LP token amount
        price_factor_num,           // Price numerator
        price_factor_denom,         // Price denominator
        decimals: Decimals {        // Precision settings
            alpha: decimals,        // For slope
            beta: decimals,         // For starting price
            quote: mint_decimals as u64,  // For SOL
        },
    };

    // Chapter 6: Setting Up Token Distribution 🎁
    // Configure token reserve
    pool.meme_reserve.tokens = DEFAULT_MAX_M as u64;  // 690M for trading
    pool.meme_reserve.mint = accs.meme_mint.key();    // Token mint address
    pool.meme_reserve.vault = accs.meme_vault.key();  // Token vault address

    // Final settings
    pool.locked = false;                        // Pool ready for trading
    pool.creator_addr = accs.sender.key();      // Sam is the creator
    pool.airdropped_tokens = airdropped_tokens; // Set airdrop amount
    pool.vesting_period = vesting_period;       // Set vesting time

    Ok(())
}
