//This module provides functionality to calculate swap amounts for a bonding pool
//without executing the actual swap. It allows previewing the expected input and 
//output amounts before performing a swap transaction.
//
//This calculates swapping Solana (SOL) to memecoins, converting the quote token
// to the project's memecoin token via the bonding curve.

use crate::models::bound::BoundPool;
use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

/// Account validation struct for getting swap amounts
#[derive(Accounts)]
pub struct GetSwapXAmt<'info> {
    /// The bonding pool account to calculate swap amounts for
    pub pool: Account<'info, BoundPool>,
    
    /// The quote token vault, must match the pool's quote reserve vault
    #[account(constraint = pool.quote_reserve.vault == quote_vault.key())]
    pub quote_vault: Account<'info, TokenAccount>,
}

/// Calculates and logs the expected swap amounts for a given input amount
///
/// # Arguments
/// * `ctx` - The context containing accounts
/// * `coin_in_amount` - The amount of input tokens to swap
/// * `coin_y_min_value` - The minimum amount of output tokens expected
///
/// # Returns
/// * `Result<()>` - Returns Ok if calculation succeeds
pub fn handle(ctx: Context<GetSwapXAmt>, coin_in_amount: u64, coin_y_min_value: u64) -> Result<()> {
    let swap_amount = ctx
        .accounts
        .pool
        .swap_amounts(coin_in_amount, coin_y_min_value, false);

    msg!(
        "swapped_in: {}\n swapped_out: {}",
        swap_amount.amount_in,
        swap_amount.amount_out
    );

    Ok(())
}
