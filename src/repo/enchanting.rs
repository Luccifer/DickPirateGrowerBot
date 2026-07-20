use anyhow::Context;
use sqlx::Row;
use teloxide::types::UserId;
use crate::repository;
use super::ChatIdKind;

/// The enchanting state of a player in a chat.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct EnchState {
    pub sharpness: i32,
    pub bless_charges: i32,
    pub attempts_left: i32,
}

#[derive(sqlx::FromRow, Debug)]
pub struct VaginaCandidate {
    pub uid: i64,
    pub name: String,
}

#[derive(sqlx::FromRow, Debug)]
pub struct GnomeVictimCandidate {
    pub uid: i64,
    pub name: String,
    pub length: i32,
}

#[derive(sqlx::FromRow, Debug)]
pub struct GnomeChat {
    pub internal_id: i64,
    pub tg_chat_id: i64,
}

// NOTE: the queries in this repository use the runtime (non-macro) sqlx API on purpose:
// this way no prepared .sqlx offline data is required for the new queries.
repository!(Enchanting,
    /// Fetches the state, creating a row and/or refilling the daily attempts if needed.
    /// `daily_attempts` is the rolled amount of attempts to grant for the new day.
    pub async fn get_or_init(&self, uid: UserId, chat_id_internal: i64, daily_attempts: i32) -> anyhow::Result<EnchState> {
        sqlx::query_as::<_, EnchState>(
            "INSERT INTO Enchanting (uid, chat_id, attempts_left, attempts_date) VALUES ($1, $2, $3, current_date)
                ON CONFLICT (chat_id, uid) DO UPDATE SET
                    attempts_left = CASE WHEN Enchanting.attempts_date < current_date THEN $3 ELSE Enchanting.attempts_left END,
                    attempts_date = current_date
                RETURNING sharpness, bless_charges, attempts_left")
            .bind(uid.0 as i64)
            .bind(chat_id_internal)
            .bind(daily_attempts)
            .fetch_one(&self.pool)
            .await
            .context(format!("couldn't init the enchanting state of {uid} in {chat_id_internal}"))
    }
,
    /// Spends one attempt (and optionally a bless charge) and sets the new sharpness.
    /// Returns false if there were no attempts (or blesses) left — protects against double-clicks.
    pub async fn apply_ench_attempt(&self, uid: UserId, chat_id_internal: i64, new_sharpness: i32, spend_bless: bool) -> anyhow::Result<bool> {
        let bless_spend = if spend_bless { 1 } else { 0 };
        let res = sqlx::query(
            "UPDATE Enchanting SET
                    attempts_left = attempts_left - 1,
                    bless_charges = bless_charges - $4,
                    sharpness = $3
                WHERE uid = $1 AND chat_id = $2 AND attempts_left > 0 AND bless_charges >= $4")
            .bind(uid.0 as i64)
            .bind(chat_id_internal)
            .bind(new_sharpness)
            .bind(bless_spend)
            .execute(&self.pool)
            .await
            .context(format!("couldn't apply an ench attempt of {uid} in {chat_id_internal}"))?;
        Ok(res.rows_affected() == 1)
    }
,
    /// Adds bless charges to the player's account (creating the row if needed).
    /// A freshly created row gets attempts_date = 'epoch' so that the first /ench
    /// of the day still grants the daily attempts (see get_or_init).
    pub async fn add_blesses(&self, uid: UserId, chat_id_internal: i64, amount: i32) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO Enchanting (uid, chat_id, bless_charges, attempts_date) VALUES ($1, $2, $3, 'epoch')
                ON CONFLICT (chat_id, uid) DO UPDATE SET bless_charges = Enchanting.bless_charges + $3")
            .bind(uid.0 as i64)
            .bind(chat_id_internal)
            .bind(amount)
            .execute(&self.pool)
            .await
            .context(format!("couldn't add {amount} blesses to {uid} in {chat_id_internal}"))?;
        Ok(())
    }
,
    /// Resets the sharpness to zero (a lost /sex battle or a bitten-off dick).
    pub async fn reset_sharpness(&self, uid: UserId, chat_id_internal: i64) -> anyhow::Result<()> {
        sqlx::query("UPDATE Enchanting SET sharpness = 0 WHERE uid = $1 AND chat_id = $2")
            .bind(uid.0 as i64)
            .bind(chat_id_internal)
            .execute(&self.pool)
            .await
            .context(format!("couldn't reset the sharpness of {uid} in {chat_id_internal}"))?;
        Ok(())
    }
,
    /// Fetches the current state without any modifications.
    pub async fn get_state(&self, uid: UserId, chat_id_internal: i64) -> anyhow::Result<Option<EnchState>> {
        sqlx::query_as::<_, EnchState>(
            "SELECT sharpness, bless_charges, attempts_left FROM Enchanting WHERE uid = $1 AND chat_id = $2")
            .bind(uid.0 as i64)
            .bind(chat_id_internal)
            .fetch_optional(&self.pool)
            .await
            .context(format!("couldn't fetch the enchanting state of {uid} in {chat_id_internal}"))
    }
,
    /// Fetches the stored name of a user by the id.
    pub async fn get_user_name(&self, uid: i64) -> anyhow::Result<Option<String>> {
        let row = sqlx::query("SELECT name FROM Users WHERE uid = $1")
            .bind(uid)
            .fetch_optional(&self.pool)
            .await
            .context(format!("couldn't fetch the name of {uid}"))?;
        row.map(|r| r.try_get("name").map_err(Into::into)).transpose()
    }
,
    /// A random player with a non-positive dick (i.e. an owner of a vagina) active during the last week.
    pub async fn get_random_vagina_owner(&self, chat_id: &ChatIdKind) -> anyhow::Result<Option<VaginaCandidate>> {
        sqlx::query_as::<_, VaginaCandidate>(
            "SELECT u.uid, u.name FROM Users u
                JOIN Dicks d USING (uid)
                JOIN Chats c ON d.chat_id = c.id
                WHERE (c.chat_id = $1::bigint OR c.chat_instance = $1::text)
                    AND d.length <= 0
                    AND d.updated_at > current_timestamp - interval '1 week'
                ORDER BY random() LIMIT 1")
            .bind(chat_id.value())
            .fetch_optional(&self.pool)
            .await
            .context(format!("couldn't get a random vagina owner in {chat_id}"))
    }
,
    /// Registers the Bless of Day; returns false if it was already given out today.
    pub async fn try_register_bless_of_day(&self, chat_id_internal: i64, winner_uid: i64, amount: i32) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "INSERT INTO Bless_Of_Day (chat_id, bless_date, winner_uid, amount) VALUES ($1, current_date, $2, $3)
                ON CONFLICT (chat_id, bless_date) DO NOTHING")
            .bind(chat_id_internal)
            .bind(winner_uid)
            .bind(amount)
            .execute(&self.pool)
            .await
            .context(format!("couldn't register the bless of day in {chat_id_internal}"))?;
        Ok(res.rows_affected() == 1)
    }
,
    /// The sum of all positive dicks in the chat — the "meat mass" driving ench chances up.
    pub async fn get_chat_meat_mass(&self, chat_id_internal: i64) -> anyhow::Result<i64> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(length), 0) AS mass FROM Dicks WHERE chat_id = $1 AND length > 0")
            .bind(chat_id_internal)
            .fetch_one(&self.pool)
            .await
            .context(format!("couldn't compute the meat mass of {chat_id_internal}"))?;
        let mass: i64 = row.try_get("mass")?;
        Ok(mass)
    }
,
    /// Chats (with known Telegram ids) which have at least one dick longer than the threshold
    /// and no gnome raid registered for the given date yet.
    pub async fn get_chats_awaiting_gnome_raid(&self, min_length: i32) -> anyhow::Result<Vec<GnomeChat>> {
        sqlx::query_as::<_, GnomeChat>(
            "SELECT c.id AS internal_id, c.chat_id AS tg_chat_id FROM Chats c
                WHERE c.chat_id IS NOT NULL
                    AND EXISTS (SELECT 1 FROM Dicks d WHERE d.chat_id = c.id AND d.length > $1)
                    AND NOT EXISTS (SELECT 1 FROM Gnome_Raids r WHERE r.chat_id = c.id AND r.raid_date = current_date)")
            .bind(min_length)
            .fetch_all(&self.pool)
            .await
            .context("couldn't fetch the chats awaiting a gnome raid")
    }
,
    /// Players of the chat eligible to be robbed by the gnomes.
    pub async fn get_gnome_victim_candidates(&self, chat_id_internal: i64, min_length: i32) -> anyhow::Result<Vec<GnomeVictimCandidate>> {
        sqlx::query_as::<_, GnomeVictimCandidate>(
            "SELECT u.uid, u.name, d.length FROM Users u
                JOIN Dicks d USING (uid)
                WHERE d.chat_id = $1 AND d.length > $2")
            .bind(chat_id_internal)
            .bind(min_length)
            .fetch_all(&self.pool)
            .await
            .context(format!("couldn't fetch gnome victim candidates in {chat_id_internal}"))
    }
,
    /// Registers a gnome raid; returns false if the chat was already raided today (a race between instances).
    pub async fn try_register_gnome_raid(&self, chat_id_internal: i64, victim_uid: i64, stolen: i32) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "INSERT INTO Gnome_Raids (chat_id, raid_date, victim_uid, stolen) VALUES ($1, current_date, $2, $3)
                ON CONFLICT (chat_id, raid_date) DO NOTHING")
            .bind(chat_id_internal)
            .bind(victim_uid)
            .bind(stolen)
            .execute(&self.pool)
            .await
            .context(format!("couldn't register a gnome raid in {chat_id_internal}"))?;
        Ok(res.rows_affected() == 1)
    }
);
