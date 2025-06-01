use crate::err::AmmError;
use crate::models::bound::BoundPool;
use crate::models::staked_lp::MemeTicket;
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

/// Account validation struct for swapping meme tokens for SOL
/// 
/// This struct validates that all required accounts are present and properly configured
/// for swapping meme tokens (X) for SOL (Y) in the bonding curve pool.
///
/// # Account Requirements
/// * `pool` - The mutable bonding curve pool account
/// * `meme_ticket` - The user's meme token ticket account, must be owned by signer
/// * `user_sol` - The user's SOL token account to receive swapped tokens
/// * `quote_vault` - The pool's SOL vault account
/// * `owner` - The signer/owner of the meme ticket
/// * `pool_signer` - PDA with authority over pool accounts
/// * `token_program` - The Solana Token Program
#[derive(Accounts)]
pub struct SwapCoinX<'info> {
    #[account(mut)]
    pub pool: Account<'info, BoundPool>,
    #[account(
        mut,
        has_one = pool,
        has_one = owner
    )]
    pub meme_ticket: Account<'info, MemeTicket>,
    #[account(mut)]
    pub user_sol: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = pool.quote_reserve.vault == quote_vault.key()
    )]
    pub quote_vault: Account<'info, TokenAccount>,
    pub owner: Signer<'info>,
    /// CHECK: pda signer
    #[account(seeds = [BoundPool::SIGNER_PDA_PREFIX, pool.key().as_ref()], bump)]
    pub pool_signer: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
}

impl<'info> SwapCoinX<'info> {
    /// Creates a CPI context for transferring SOL tokens to the user
    ///
    /// This helper function prepares the CPI context needed to transfer SOL tokens
    /// from the pool's quote vault to the user's SOL token account.
    ///
    /// # Returns
    /// * `CpiContext` - The context for the token transfer CPI
    fn send_tokens_to_user(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.quote_vault.to_account_info(),
            to: self.user_sol.to_account_info(),
            authority: self.pool_signer.to_account_info(),
        };

        let cpi_program = self.token_program.to_account_info();
        CpiContext::new(cpi_program, cpi_accounts)
    }
}

/// Handles the swap of meme tokens for SOL using the bonding curve
///
/// This function processes a swap where a user trades their meme tokens for SOL.
/// The swap follows the bonding curve pricing model, ensuring fair and predictable pricing.
///
/// # Arguments
/// * `ctx` - The context containing all required accounts
/// * `coin_in_amount` - The amount of meme tokens to swap
/// * `coin_y_min_value` - The minimum amount of SOL to receive (slippage protection)
///
/// # Returns
/// * `Result<()>` - Result indicating success or containing error
///
/// # Errors
/// * `AmmError::NoZeroTokens` - If attempting to swap 0 tokens
/// * `AmmError::TicketTokensLocked` - If the meme tokens are still locked
/// * `AmmError::NotEnoughTicketTokens` - If user has insufficient tokens
/// * `AmmError::PoolIsLocked` - If the pool is currently locked
pub fn handle(ctx: Context<SwapCoinX>, coin_in_amount: u64, coin_y_min_value: u64) -> Result<()> {
    let accs = ctx.accounts;

    if coin_in_amount == 0 {
        return Err(error!(AmmError::NoZeroTokens));
    }

    let user_ticket = &mut accs.meme_ticket;

    if !user_ticket.is_unlocked() {
        return Err(error!(AmmError::TicketTokensLocked));
    }

    if coin_in_amount > user_ticket.amount {
        return Err(error!(AmmError::NotEnoughTicketTokens));
    }

    let pool_state = &mut accs.pool;

    if pool_state.locked {
        return Err(error!(AmmError::PoolIsLocked));
    }

    let swap_amount = pool_state.swap_amounts(coin_in_amount, coin_y_min_value, false);

    pool_state.admin_fees_meme += swap_amount.admin_fee_in;
    pool_state.admin_fees_quote += swap_amount.admin_fee_out;

    pool_state.meme_reserve.tokens += swap_amount.amount_in;
    pool_state.quote_reserve.tokens -= swap_amount.amount_out + swap_amount.admin_fee_out;

    user_ticket.amount -= coin_in_amount;
    user_ticket.vesting.notional -= coin_in_amount;

    let seeds = &[
        BoundPool::SIGNER_PDA_PREFIX,
        &accs.pool.key().to_bytes()[..],
        &[ctx.bumps.pool_signer],
    ];

    let signer_seeds = &[&seeds[..]];

    token::transfer(
        accs.send_tokens_to_user().with_signer(signer_seeds),
        swap_amount.amount_out,
    )
    .unwrap();

    msg!(
        "swapped_in: {}\n swapped_out: {}",
        swap_amount.amount_in,
        swap_amount.amount_out
    );

    Ok(())
}
