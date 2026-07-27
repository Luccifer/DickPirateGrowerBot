use sqlx::{Pool, Postgres};
use rust_i18n::t;
use teloxide::Bot;
use teloxide::payloads::SendMessageSetters;
use teloxide::requests::Requester;
use teloxide::types::ChatId;
use teloxide::types::ParseMode::Html;

/// Announces the farm rollback (migration 23) in every group chat, exactly once.
/// The migration inserts a row into Farm_Rollbacks with announced = false; this task
/// retries every minute until at least one delivery succeeds, then flips the flag.
/// Flaky networks lose to persistence.
pub fn spawn_farm_rollback_announcer(bot: Bot, pool: Pool<Postgres>) {
    tokio::spawn(async move {
        // let the dispatcher and the network settle first
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        loop {
            match announce_farm_rollback(&bot, &pool).await {
                Ok(true) => break,  // nothing pending, or the news is out
                Ok(false) => log::warn!("the farm rollback announcement wasn't delivered, retrying in 60 seconds"),
                Err(e) => log::error!("the farm rollback announcement failed: {e}; retrying in 60 seconds"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

/// Returns Ok(true) when there is nothing left to announce (either no pending
/// rollbacks, or the message has just been delivered and the flag is flipped).
async fn announce_farm_rollback(bot: &Bot, pool: &Pool<Postgres>) -> anyhow::Result<bool> {
    let pending: Vec<i32> = sqlx::query_scalar("SELECT id FROM Farm_Rollbacks WHERE announced = false")
        .fetch_all(pool).await?;
    if pending.is_empty() {
        return Ok(true)
    }

    let locale = std::env::var("GNOMES_LOCALE").unwrap_or_else(|_| "ru".to_owned());
    let text = t!("commands.gnomes.rollback", locale = &locale).to_string();

    // negative ids are groups and supergroups; private chats are spared the auditor's rage
    let chat_ids: Vec<i64> = sqlx::query_scalar("SELECT chat_id FROM Chats WHERE chat_id < 0")
        .fetch_all(pool).await?;
    let mut delivered = false;
    for tg_chat_id in chat_ids {
        match bot.send_message(ChatId(tg_chat_id), text.clone()).parse_mode(Html).await {
            Ok(_) => delivered = true,
            Err(e) => log::error!("couldn't deliver the rollback announcement to the chat {tg_chat_id}: {e}"),
        }
    }
    if delivered {
        sqlx::query("UPDATE Farm_Rollbacks SET announced = true WHERE announced = false")
            .execute(pool).await?;
        log::info!("the farm rollback has been announced");
    }
    Ok(delivered)
}
