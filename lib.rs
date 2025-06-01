use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};
use crate::errors::AmmError;
use crate::models::bound::BoundPool;
use crate::models::points_epoch::PointsEpoch;
use crate::consts::{POINTS_MINT, POINTS_PDA};
use std::cmp::min;

// Error codes for the Monopoly game system
#[error_code]
pub enum MonopolyError {
    #[msg("Output amount is less than minimum required")]
    InsufficientOutputAmount,
    
    #[msg("Invalid property position")]
    InvalidPropertyPosition,
    
    #[msg("Insufficient SOL balance")]
    InsufficientBalance,
    
    #[msg("Invalid ticket number")]
    InvalidTicketNumber,
    
    #[msg("Points distribution failed")]
    PointsDistributionFailed,
}

// Account validation struct with all required accounts for property stake purchase
#[derive(Accounts)]
#[instruction(position: u8, ticket_number: u64)]
pub struct BuyPropertyStake<'info> {
    // Property stake account - stores individual stake information
    #[account(
        init,
        payer = owner,
        space = 8 + PropertyStake::INIT_SPACE,
        seeds = [
            b"property_stake",
            position.to_le_bytes().as_ref(),
            owner.key().as_ref(),
            ticket_number.to_le_bytes().as_ref()
        ],
        bump
    )]
    pub property_stake: Account<'info, PropertyStake>,

    // Property state account - stores aggregate property information
    #[account(
        init,
        payer = owner,
        space = 8 + PropertyState::INIT_SPACE,
        seeds = [b"property_state", position.to_le_bytes().as_ref()],
        bump
    )]
    pub property_state: Account<'info, PropertyState>,

    // Bonding curve pool account
    #[account(mut)]
    pub pool: Account<'info, BoundPool>,

    // Pool's SOL vault
    #[account(
        mut,
        constraint = pool.quote_reserve.vault == quote_vault.key()
    )]
    pub quote_vault: Account<'info, TokenAccount>,

    // User's SOL account
    #[account(mut)]
    pub user_sol: Account<'info, TokenAccount>,

    // User's points account
    #[account(
        mut,
        token::mint = points_mint,
        token::authority = owner,
    )]
    pub user_points: Account<'info, TokenAccount>,

    // Optional referrer's points account
    #[account(
        mut,
        token::mint = points_mint,
        constraint = referrer_points.owner != user_points.owner
    )]
    pub referrer_points: Option<Account<'info, TokenAccount>>,

    // Points distribution account
    points_epoch: Account<'info, PointsEpoch>,

    // Points mint account
    #[account(mut, constraint = points_mint.key() == POINTS_MINT.key())]
    pub points_mint: Account<'info, Mint>,

    // PDA holding points to distribute
    #[account(
        mut,
        token::mint = points_mint,
        token::authority = points_pda
    )]
    pub points_acc: Account<'info, TokenAccount>,

    // Transaction signer (user)
    #[account(mut)]
    pub owner: Signer<'info>,

    // PDA for points distribution
    /// CHECK: Safe - PDA for points
    #[account(seeds = [POINTS_PDA], bump)]
    pub points_pda: AccountInfo<'info>,

    // PDA for pool operations
    /// CHECK: Safe - Pool PDA
    #[account(seeds = [BoundPool::SIGNER_PDA_PREFIX, pool.key().as_ref()], bump)]
    pub pool_signer_pda: AccountInfo<'info>,

    // Required programs
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

// Main handler function for buying property stakes
pub fn buy_property_stake(
    ctx: Context<BuyPropertyStake>,
    position: u8,
    coin_in_amount: u64,
    coin_x_min_value: u64,
    ticket_number: u64,
) -> Result<()> {
    // Step 1: Validate basic parameters
    require!(position < 40, MonopolyError::InvalidPropertyPosition); // Monopoly board has 40 spaces
    require!(coin_in_amount > 0, AmmError::NoZeroTokens);
    require!(!ctx.accounts.pool.locked, AmmError::PoolIsLocked);

    
    
    let swap_amount = swap_calc_accounts
        .pool
        .swap_amounts(coin_in_amount, coin_x_min_value, true);

    // Validate minimum output amount
    require!(
        swap_amount.amount_out >= coin_x_min_value,
        MonopolyError::InsufficientOutputAmount
    );

    // Step 3: Record current timestamp
    let clock = Clock::get()?;
    let current_timestamp = clock.unix_timestamp;

    // Step 4: Execute SOL transfer
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.user_sol.to_account_info(),
                to: ctx.accounts.quote_vault.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        swap_amount.amount_in + swap_amount.admin_fee_in,
    )?;

    // Step 5: Handle points distribution
    let point_pda: &[&[u8]] = &[POINTS_PDA, &[ctx.bumps.points_pda]];
    let point_pda_seeds = &[&point_pda[..]];

    let available_points = ctx.accounts.points_acc.amount;
    let points = get_swap_points(
        swap_amount.amount_in + swap_amount.admin_fee_in,
        &ctx.accounts.points_epoch
    );
    let clamped_points = min(available_points, points);

    if clamped_points > 0 {
        // Transfer points to user
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.points_acc.to_account_info(),
                    to: ctx.accounts.user_points.to_account_info(),
                    authority: ctx.accounts.points_pda.to_account_info(),
                },
                point_pda_seeds,
            ),
            clamped_points,
        )?;

        // Handle referral points if provided
        if let Some(referrer) = &ctx.accounts.referrer_points {
            let referral_points = clamped_points.mul_div_floor(25_000, 100_000)?;
            if referral_points > 0 {
                token::transfer(
                    CpiContext::new_with_signer(
                        ctx.accounts.token_program.to_account_info(),
                        Transfer {
                            from: ctx.accounts.points_acc.to_account_info(),
                            to: referrer.to_account_info(),
                            authority: ctx.accounts.points_pda.to_account_info(),
                        },
                        point_pda_seeds,
                    ),
                    referral_points,
                )?;
            }
        }
    }

    // Step 6: Update pool state
    let pool = &mut ctx.accounts.pool;
    pool.admin_fees_quote += swap_amount.admin_fee_in;
    pool.admin_fees_meme += swap_amount.admin_fee_out;
    pool.quote_reserve.tokens += swap_amount.amount_in;
    pool.meme_reserve.tokens -= swap_amount.amount_out + swap_amount.admin_fee_out;

    if pool.meme_reserve.tokens == 0 {
        pool.locked = true;
    }

    // Step 7: Create property stake record
    let property_stake = &mut ctx.accounts.property_stake;
    property_stake.property_position = position;
    property_stake.owner = ctx.accounts.owner.key();
    property_stake.stake_amount = swap_amount.amount_out;
    property_stake.purchase_timestamp = current_timestamp;

    // Step 8: Update property state
    let property_state = &mut ctx.accounts.property_state;
    property_state.total_investment += swap_amount.amount_out;
    property_state.stake_count += 1;

    // Log transaction details
    msg!(
        "Property stake created - Position: {}, SOL invested: {}, Memecoin received: {}, Points earned: {}",
        position,
        coin_in_amount,
        swap_amount.amount_out,
        clamped_points
    );

    Ok(())
}