use crate::error::{AppError, Result};
use crate::models::{CreateUser, PublicUser, UpdateUser, User, UserStatus};
use argon2::{
    password_hash::{rand_core::OsRng, SaltString},
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct UserService {
    db: PgPool,
}

impl UserService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateUser) -> Result<User> {
        // Check if username or email already exists
        let existing = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM users WHERE username = $1 OR email = $2)"#,
            input.username,
            input.email
        )
        .fetch_one(&self.db)
        .await?;

        if existing.unwrap_or(false) {
            return Err(AppError::Conflict(
                "Username or email already exists".to_string(),
            ));
        }

        // Hash password
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(input.password.as_bytes(), &salt)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Password hashing failed: {}", e)))?
            .to_string();

        let user = sqlx::query_as!(
            User,
            r#"
            INSERT INTO users (id, username, display_name, email, password_hash, status, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
            RETURNING id, username, display_name, email, password_hash, avatar_url,
                      status as "status: UserStatus", custom_status, created_at, updated_at
            "#,
            Uuid::new_v4(),
            input.username,
            input.display_name,
            input.email,
            password_hash,
            UserStatus::Offline as UserStatus,
        )
        .fetch_one(&self.db)
        .await?;

        Ok(user)
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<User> {
        let user = sqlx::query_as!(
            User,
            r#"
            SELECT id, username, display_name, email, password_hash, avatar_url,
                   status as "status: UserStatus", custom_status, created_at, updated_at
            FROM users WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

        Ok(user)
    }

    pub async fn get_by_username(&self, username: &str) -> Result<User> {
        let user = sqlx::query_as!(
            User,
            r#"
            SELECT id, username, display_name, email, password_hash, avatar_url,
                   status as "status: UserStatus", custom_status, created_at, updated_at
            FROM users WHERE username = $1
            "#,
            username
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

        Ok(user)
    }

    pub async fn verify_credentials(&self, username: &str, password: &str) -> Result<User> {
        let user = self.get_by_username(username).await?;

        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Invalid password hash: {}", e)))?;

        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| AppError::Unauthorized)?;

        Ok(user)
    }

    pub async fn update(&self, id: Uuid, input: UpdateUser) -> Result<User> {
        let user = sqlx::query_as!(
            User,
            r#"
            UPDATE users
            SET display_name = COALESCE($2, display_name),
                avatar_url = COALESCE($3, avatar_url),
                custom_status = COALESCE($4, custom_status),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, username, display_name, email, password_hash, avatar_url,
                      status as "status: UserStatus", custom_status, created_at, updated_at
            "#,
            id,
            input.display_name,
            input.avatar_url,
            input.custom_status
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

        Ok(user)
    }

    pub async fn update_status(&self, id: Uuid, status: UserStatus) -> Result<()> {
        sqlx::query!(
            "UPDATE users SET status = $2, updated_at = NOW() WHERE id = $1",
            id,
            status as UserStatus
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    pub async fn get_friends(&self, user_id: Uuid) -> Result<Vec<PublicUser>> {
        let friends = sqlx::query_as!(
            PublicUser,
            r#"
            SELECT u.id, u.username, u.display_name, u.avatar_url,
                   u.status as "status: UserStatus", u.custom_status
            FROM users u
            INNER JOIN friendships f ON (f.user1_id = u.id OR f.user2_id = u.id)
            WHERE (f.user1_id = $1 OR f.user2_id = $1) AND u.id != $1
            AND f.status = 'accepted'
            "#,
            user_id
        )
        .fetch_all(&self.db)
        .await?;

        Ok(friends)
    }
}
