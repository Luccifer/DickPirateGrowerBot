use std::ops::RangeInclusive;
use std::sync::OnceLock;

use anyhow::anyhow;
use rand::Rng;
use rand::rngs::OsRng;
use rust_i18n::t;
use teloxide::Bot;
use teloxide::macros::BotCommands;
use teloxide::payloads::SendMessageSetters;
use teloxide::requests::Requester;
use teloxide::types::{CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message, ReplyMarkup, UserId};
use teloxide::types::ParseMode::Html;

use crate::{config, reply_html, repo};
use crate::domain::{LanguageCode, Username};
use crate::handlers::{CallbackResult, HandlerResult, reply_html, send_error_callback_answer, utils};
use crate::handlers::utils::callbacks;
use crate::handlers::utils::callbacks::{CallbackDataWithPrefix, InvalidCallbackDataBuilder};
use crate::repo::Repositories;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum EnchCommands {
    #[command(description = "ench")]
    Ench,
    #[command(description = "enchb")]
    Enchb,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum BlessCommands {
    #[command(description = "bless")]
    Bless,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum SexCommands {
    #[command(description = "sex")]
    Sex,
}

pub struct EnchConfig {
    attempts_per_day: RangeInclusive<i32>,
    safe_level: i32,
    difficulty: f64,
    chance_min: u32,
    chance_max: u32,
    bless_amount: RangeInclusive<i32>,
    gnomes_min_length: i32,
    gnomes_steal: RangeInclusive<i32>,
    gnomes_raid_hour_utc: u32,
    gnomes_locale: String,
}

impl EnchConfig {
    pub fn instance() -> &'static EnchConfig {
        static INSTANCE: OnceLock<EnchConfig> = OnceLock::new();
        INSTANCE.get_or_init(|| EnchConfig {
            attempts_per_day: parse_range_env("ENCH_ATTEMPTS_PER_DAY", 20..=20),
            safe_level: config::get_env_value_or_default("ENCH_SAFE_LEVEL", 3),
            difficulty: config::get_env_value_or_default("ENCH_DIFFICULTY", 5.0),
            chance_min: config::get_env_value_or_default("ENCH_CHANCE_MIN", 5),
            chance_max: config::get_env_value_or_default("ENCH_CHANCE_MAX", 100),
            bless_amount: parse_range_env("BLESS_DAILY_AMOUNT", 1..=3),
            gnomes_min_length: config::get_env_value_or_default("GNOMES_MIN_LENGTH", 20),
            gnomes_steal: parse_range_env("GNOMES_STEAL", 1..=10),
            gnomes_raid_hour_utc: config::get_env_value_or_default("GNOMES_RAID_HOUR_UTC", 6),
            gnomes_locale: config::get_env_value_or_default("GNOMES_LOCALE", "ru".to_owned()),
        })
    }
}

/// Parses either a fixed number ("20") or an inclusive range ("10..20") from an env var.
fn parse_range_env(key: &str, default: RangeInclusive<i32>) -> RangeInclusive<i32> {
    let raw = match std::env::var(key) {
        Ok(v) => v,
        Err(_) => {
            log::warn!("no value was found for an optional environment variable {key}, using the default value {}..{}",
                default.start(), default.end());
            return default
        }
    };
    let parsed = if let Some((from, to)) = raw.split_once("..") {
        from.trim().parse::<i32>().ok()
            .zip(to.trim().parse::<i32>().ok())
            .filter(|(from, to)| from <= to && *from >= 0)
            .map(|(from, to)| from..=to)
    } else {
        raw.trim().parse::<i32>().ok()
            .filter(|v| *v >= 0)
            .map(|v| v..=v)
    };
    parsed.unwrap_or_else(|| {
        log::warn!("invalid value '{raw}' of the {key} environment variable, using the default value {}..{}",
            default.start(), default.end());
        default
    })
}

fn roll_range(range: &RangeInclusive<i32>) -> i32 {
    if range.start() == range.end() {
        *range.start()
    } else {
        OsRng.gen_range(range.clone())
    }
}

/// The chance (in percents) to successfully enchant the vagina up to `target_level`.
/// Driven by the total mass of positive dicks in the chat: the more meat around, the easier it goes.
/// Non-linear: the difficulty grows quadratically with the level.
fn ench_chance(cfg: &EnchConfig, meat_mass: i64, target_level: i32) -> u32 {
    if target_level <= cfg.safe_level {
        return 100
    }
    let mass = meat_mass.max(0) as f64;
    let over = (target_level - cfg.safe_level) as f64;
    let chance = 100.0 * mass / (mass + over * over * cfg.difficulty);
    (chance as u32).clamp(cfg.chance_min, cfg.chance_max)
}

pub async fn ench_cmd_handler(bot: Bot, msg: Message, cmd: EnchCommands, repos: Repositories) -> HandlerResult {
    let cfg = EnchConfig::instance();
    let from = msg.from.as_ref().ok_or(anyhow!("unexpected absence of a FROM field"))?;
    let lang_code = LanguageCode::from_user(from);
    let chat_id = msg.chat.id.into();
    let name = utils::get_full_name(from);
    repos.users.create_or_update(from.id, &name).await?;
    let chat_internal = repos.chats.upsert_chat(&chat_id).await?;

    let length = repos.dicks.fetch_length(from.id, &chat_id.kind()).await?;
    if length > 0 {
        let answer = t!("commands.ench.errors.no_vagina", locale = &lang_code, length = length);
        reply_html!(bot, msg, answer);
        return Ok(())
    }

    let daily_attempts = roll_range(&cfg.attempts_per_day);
    let state = repos.enchanting.get_or_init(from.id, chat_internal, daily_attempts).await?;
    let with_bless = matches!(cmd, EnchCommands::Enchb);

    let answer = if state.attempts_left <= 0 {
        let time_left = utils::date::get_time_till_next_day_string(&lang_code);
        format!("{}{}", t!("commands.ench.errors.no_attempts", locale = &lang_code), time_left)
    } else if with_bless && state.bless_charges <= 0 {
        t!("commands.ench.errors.no_blesses", locale = &lang_code).to_string()
    } else {
        let target = state.sharpness + 1;
        let meat_mass = repos.enchanting.get_chat_meat_mass(chat_internal).await?;
        let chance = ench_chance(cfg, meat_mass, target);
        let success = chance >= 100 || OsRng.gen_ratio(chance, 100);
        let new_sharpness = if success {
            target
        } else if with_bless {
            state.sharpness
        } else {
            0
        };
        let applied = repos.enchanting.apply_ench_attempt(from.id, chat_internal, new_sharpness, with_bless).await?;
        if !applied {
            t!("commands.ench.errors.no_attempts", locale = &lang_code).to_string()
        } else {
            let left = state.attempts_left - 1;
            let key = match (success, with_bless) {
                (true, false) => "commands.ench.success",
                (true, true) => "commands.ench.success_bless",
                (false, false) => "commands.ench.fail",
                (false, true) => "commands.ench.fail_bless",
            };
            let main_part = t!(key, locale = &lang_code,
                level = new_sharpness, chance = chance, blesses = (state.bless_charges - if with_bless { 1 } else { 0 }));
            let left_part = t!("commands.ench.attempts_left", locale = &lang_code, left = left);
            format!("{main_part}\n{left_part}")
        }
    };
    reply_html!(bot, msg, answer);
    Ok(())
}

pub async fn bless_cmd_handler(bot: Bot, msg: Message, repos: Repositories) -> HandlerResult {
    let cfg = EnchConfig::instance();
    let from = msg.from.as_ref().ok_or(anyhow!("unexpected absence of a FROM field"))?;
    let lang_code = LanguageCode::from_user(from);
    let chat_id = msg.chat.id.into();
    let chat_internal = repos.chats.upsert_chat(&chat_id).await?;

    let answer = match repos.enchanting.get_random_vagina_owner(&chat_id.kind()).await? {
        None => t!("commands.bless.no_candidates", locale = &lang_code).to_string(),
        Some(winner) => {
            let amount = roll_range(&cfg.bless_amount);
            let registered = repos.enchanting.try_register_bless_of_day(chat_internal, winner.uid, amount).await?;
            if registered {
                repos.enchanting.add_blesses(UserId(winner.uid as u64), chat_internal, amount).await?;
                let name = Username::new(winner.name).escaped();
                let main_part = t!("commands.bless.result", locale = &lang_code,
                    uid = winner.uid, name = name, amount = amount);
                let time_left = utils::date::get_time_till_next_day_string(&lang_code);
                format!("{main_part}{time_left}")
            } else {
                t!("commands.bless.already_chosen", locale = &lang_code).to_string()
            }
        }
    };
    reply_html!(bot, msg, answer);
    Ok(())
}

#[derive(derive_more::Display)]
#[display("{initiator}:{sharpness}")]
pub struct SexCallbackData {
    initiator: UserId,
    sharpness: u16,
}

impl CallbackDataWithPrefix for SexCallbackData {
    fn prefix() -> &'static str {
        "sex"
    }
}

impl TryFrom<String> for SexCallbackData {
    type Error = callbacks::InvalidCallbackData;

    fn try_from(data: String) -> Result<Self, Self::Error> {
        let err = InvalidCallbackDataBuilder(&data);
        let mut parts = data.split(':');
        let initiator = callbacks::parse_part(&mut parts, &err, "uid").map(UserId)?;
        let sharpness: u16 = callbacks::parse_part(&mut parts, &err, "sharpness")?;
        Ok(Self { initiator, sharpness })
    }
}

pub async fn sex_cmd_handler(bot: Bot, msg: Message, repos: Repositories) -> HandlerResult {
    let from = msg.from.as_ref().ok_or(anyhow!("unexpected absence of a FROM field"))?;
    let lang_code = LanguageCode::from_user(from);
    let chat_id = msg.chat.id.into();
    let chat_internal = repos.chats.upsert_chat(&chat_id).await?;

    let length = repos.dicks.fetch_length(from.id, &chat_id.kind()).await?;
    if length > 0 {
        let answer = t!("commands.sex.errors.no_vagina", locale = &lang_code);
        reply_html!(bot, msg, answer);
        return Ok(())
    }
    let sharpness = repos.enchanting.get_state(from.id, chat_internal).await?
        .map(|s| s.sharpness)
        .unwrap_or(0);
    if sharpness <= 0 {
        let answer = t!("commands.sex.errors.not_sharpened", locale = &lang_code);
        reply_html!(bot, msg, answer);
        return Ok(())
    }

    let name = utils::get_full_name(from);
    let text = t!("commands.sex.start", locale = &lang_code,
        name = name.escaped(), sharpness = sharpness).to_string();
    let btn_data = SexCallbackData { initiator: from.id, sharpness: sharpness as u16 }.to_data_string();
    let btn = InlineKeyboardButton::callback(t!("commands.sex.button", locale = &lang_code), btn_data);
    let keyboard = InlineKeyboardMarkup::new(vec![vec![btn]]);

    let mut request = reply_html(bot, &msg, text);
    request.reply_markup.replace(ReplyMarkup::InlineKeyboard(keyboard));
    request.await?;
    Ok(())
}

#[inline]
pub fn sex_callback_filter(query: CallbackQuery) -> bool {
    SexCallbackData::check_prefix(query)
}

pub async fn sex_callback_handler(bot: Bot, query: CallbackQuery, repos: Repositories) -> HandlerResult {
    let data = SexCallbackData::parse(&query)?;
    let defender = &query.from;
    let lang_code = LanguageCode::from_user(defender);
    if data.initiator == defender.id {
        return send_error_callback_answer(bot, query, "commands.sex.errors.same_person").await;
    }
    let chat_id: repo::ChatIdPartiality = match query.message.as_ref().map(|msg| msg.chat().id) {
        Some(chat_id) => chat_id.into(),
        None => return send_error_callback_answer(bot, query, "commands.sex.errors.expired").await
    };
    let chat_internal = repos.chats.upsert_chat(&chat_id).await?;

    // re-check the actual state of the initiator's vagina
    let actual_sharpness = repos.enchanting.get_state(data.initiator, chat_internal).await?
        .map(|s| s.sharpness)
        .unwrap_or(0);
    if actual_sharpness <= 0 || actual_sharpness != data.sharpness as i32 {
        return send_error_callback_answer(bot, query, "commands.sex.errors.expired").await;
    }
    let sharpness = actual_sharpness;

    let defender_length = repos.dicks.fetch_length(defender.id, &chat_id.kind()).await?;
    if defender_length < sharpness {
        return send_error_callback_answer(bot, query, "commands.sex.errors.too_short").await;
    }

    let initiator_name = repos.enchanting.get_user_name(data.initiator.0 as i64).await?
        .map(|name| Username::new(name).escaped())
        .unwrap_or_else(|| "???".to_owned());
    let defender_name = utils::get_full_name(defender).escaped();

    let vagina_wins = OsRng.gen_ratio(1, 2);
    let text = if vagina_wins {
        // the vagina bites the dick off and becomes a dick of that very length
        let defender_res = repos.dicks.grow_no_attempts_check(&chat_id.kind(), defender.id, -sharpness).await?;
        let initiator_length = repos.dicks.fetch_length(data.initiator, &chat_id.kind()).await?;
        repos.dicks.grow_no_attempts_check(&chat_id.kind(), data.initiator, sharpness - initiator_length).await?;
        repos.enchanting.reset_sharpness(data.initiator, chat_internal).await?;
        t!("commands.sex.vagina_wins", locale = &lang_code,
            initiator_uid = data.initiator.0, initiator_name = initiator_name,
            defender_uid = defender.id.0, defender_name = defender_name,
            sharpness = sharpness, defender_length = defender_res.new_length).to_string()
    } else {
        // the dick stands its ground: the vagina is dulled, the winner gets half the sharpness in blesses
        repos.enchanting.reset_sharpness(data.initiator, chat_internal).await?;
        let blesses = sharpness / 2;
        if blesses > 0 {
            repos.enchanting.add_blesses(defender.id, chat_internal, blesses).await?;
        }
        t!("commands.sex.dick_wins", locale = &lang_code,
            initiator_uid = data.initiator.0, initiator_name = initiator_name,
            defender_uid = defender.id.0, defender_name = defender_name,
            blesses = blesses).to_string()
    };
    CallbackResult::EditMessage(text, None).apply(bot, query).await?;
    Ok(())
}

/// Spawns the background scheduler of the gnome raids: every day at GNOMES_RAID_HOUR_UTC
/// one player with a dick longer than GNOMES_MIN_LENGTH per chat gets robbed by the gnomes.
pub fn spawn_gnome_scheduler(bot: Bot, repos: Repositories) {
    tokio::spawn(async move {
        let cfg = EnchConfig::instance();
        loop {
            let now = chrono::Utc::now();
            let today_raid = now.date_naive()
                .and_hms_opt(cfg.gnomes_raid_hour_utc.min(23), 0, 0)
                .expect("a valid raid time");
            if now.naive_utc() >= today_raid {
                // the raid hour has already passed today: catch up (chats raided today are filtered out)
                if let Err(e) = run_gnome_raids(&bot, &repos, cfg).await {
                    log::error!("the gnome raid failed: {e}");
                }
            }
            let next_raid = if now.naive_utc() < today_raid {
                today_raid
            } else {
                today_raid + chrono::Duration::days(1)
            };
            let sleep_for = (next_raid - now.naive_utc())
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(3600));
            tokio::time::sleep(sleep_for).await;
            if let Err(e) = run_gnome_raids(&bot, &repos, cfg).await {
                log::error!("the gnome raid failed: {e}");
            }
        }
    });
}

async fn run_gnome_raids(bot: &Bot, repos: &Repositories, cfg: &EnchConfig) -> anyhow::Result<()> {
    let chats = repos.enchanting.get_chats_awaiting_gnome_raid(cfg.gnomes_min_length).await?;
    log::info!("the gnomes are going out: {} chat(s) to raid", chats.len());
    for chat in chats {
        if let Err(e) = raid_one_chat(bot, repos, cfg, &chat).await {
            log::error!("the gnomes failed to raid the chat {}: {e}", chat.tg_chat_id);
        }
    }
    Ok(())
}

async fn raid_one_chat(bot: &Bot, repos: &Repositories, cfg: &EnchConfig, chat: &repo::GnomeChat) -> anyhow::Result<()> {
    let candidates = repos.enchanting.get_gnome_victim_candidates(chat.internal_id, cfg.gnomes_min_length).await?;
    if candidates.is_empty() {
        return Ok(())
    }
    // a progressive and fair roulette: the weight is the excess over the threshold
    let total_weight: i64 = candidates.iter()
        .map(|c| (c.length - cfg.gnomes_min_length) as i64 + 1)
        .sum();
    let mut roll = OsRng.gen_range(0..total_weight);
    let mut victim = &candidates[0];
    for candidate in &candidates {
        let weight = (candidate.length - cfg.gnomes_min_length) as i64 + 1;
        if roll < weight {
            victim = candidate;
            break;
        }
        roll -= weight;
    }

    let stolen = roll_range(&cfg.gnomes_steal).min(victim.length);
    if stolen <= 0 {
        return Ok(())
    }
    let registered = repos.enchanting.try_register_gnome_raid(chat.internal_id, victim.uid, stolen).await?;
    if !registered {
        return Ok(())
    }

    let chat_id_kind: repo::ChatIdKind = ChatId(chat.tg_chat_id).into();
    let result = repos.dicks.grow_no_attempts_check(&chat_id_kind, UserId(victim.uid as u64), -stolen).await?;

    let name = Username::new(victim.name.clone()).escaped();
    let text = t!("commands.gnomes.raid", locale = &cfg.gnomes_locale,
        uid = victim.uid, name = name, stolen = stolen, length = result.new_length);
    bot.send_message(ChatId(chat.tg_chat_id), text)
        .parse_mode(Html)
        .await?;
    Ok(())
}
