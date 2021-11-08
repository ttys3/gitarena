use actix_web::HttpRequest;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use bstr::BString;
use chrono::{DateTime, FixedOffset, LocalResult, TimeZone, Utc};
use git2::{Signature as LibGit2Signature, Time as LibGit2Time};
use git_repository::actor::{Sign, Signature as GitoxideSignature, Time as GitoxideTime};
use log::warn;
use qstring::QString;
use sqlx::{Executor, Postgres};

pub(crate) trait HttpRequestExtensions {
    /// Gets a specific header from the current request.
    ///
    /// This function gets a specific [http header][header] from the current request.
    /// If the requested header does not exist in the current request or is not valid utf-8, returns `None`.
    /// This method does not allocate but instead returns a `&str`.
    ///
    /// # Example
    ///
    /// ```
    /// # let request = actix_web::test::TestRequest::with_header("content-type", "text/plain").to_http_request();
    ///
    /// use crate::prelude::*;
    ///
    /// let content_type = request.get_header("content-type");
    /// assert_eq!(content_type, Some("text/plain"));
    /// ```
    ///
    /// [header]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers
    fn get_header<S: AsRef<str>>(&self, header: S) -> Option<&str>;

    /// Gets a [QString](qstring::QString) built from the current request.
    ///
    /// This function is a shorthand for `QString::from(request.query_string())`. It is
    /// guaranteed to not fail or panic. If no query string was sent with the request,
    /// a empty QString struct is returned. This method will always allocate.
    ///
    /// # Example
    ///
    /// ```
    /// # let request = actix_web::test::TestRequest::param("v", "BXB26PzV31k").to_http_request();
    ///
    /// use crate::prelude::*;
    ///
    /// let query_string = request.q_string();
    /// assert_eq!(query_string.get("v"), Some("BXB26PzV31k"));
    /// ```
    fn q_string(&self) -> QString;
}

impl HttpRequestExtensions for HttpRequest {
    fn get_header<S: AsRef<str>>(&self, header: S) -> Option<&str> {
        self.headers().get(header.as_ref())?.to_str().ok()
    }

    fn q_string(&self) -> QString {
        QString::from(self.query_string())
    }
}

pub(crate) trait LibGit2TimeExtensions {
    /// Tries to convert from `git2` [Time][time] into `chrono` [DateTime][datetime].
    ///
    /// The returned [DateTime][datetime] timezone is [FixedOffset](chrono::FixedOffset) with
    /// the offset provided by [Time][time]. In case the conversation yields an [ambiguous result](chrono::offset::LocalResult::Ambiguous)
    /// a warning is logged and the smaller of the two ambiguous results is returned.
    ///
    /// This method will fail and return an [Error](anyhow::Error) if `offset` is out of bounds (>24 hours) or
    /// if `seconds` is out of bounds (>[i64::MAX](i64::MAX)).
    ///
    /// # Panics
    ///
    /// This function panics if this `git2` [Time][time]'s `sign()` method returns neither `'+'` or `'-'`.
    /// That would indicate a incorrect implementation of [Time][time], as a time offset can only ever be
    /// positive or negative.
    ///
    /// # Example
    ///
    /// ```
    /// # let commit_time = git2::time::Time::new(0, 0);
    ///
    /// use crate::prelude::*;
    ///
    /// let date_time = commit_time.try_as_chrono()?;
    /// assert_eq!(0, date_time.timestamp());
    ///
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// [time]: git2::Time
    /// [datetime]: chrono::DateTime
    fn try_as_chrono(&self) -> Result<DateTime<FixedOffset>>;
}

impl LibGit2TimeExtensions for LibGit2Time {
    fn try_as_chrono(&self) -> Result<DateTime<FixedOffset>> {
        let abs_offset_minutes = self.offset_minutes().abs();
        let abs_offset_seconds = abs_offset_minutes * 60;

        let offset = match self.sign() {
            '+' => FixedOffset::east_opt(abs_offset_seconds).ok_or_else(|| anyhow!("Offset out of bounds"))?,
            '-' => FixedOffset::west_opt(abs_offset_seconds).ok_or_else(|| anyhow!("Offset out of bounds"))?,
            _ => unreachable!("unexpected sign: {}", self.sign())
        };

        match offset.timestamp_opt(self.seconds(), 0) {
            LocalResult::Single(date_time) => Ok(date_time),
            LocalResult::Ambiguous(min, max) => {
                warn!("Received ambiguous result for commit: {} and {}", &min, &max);
                Ok(min)
            },
            LocalResult::None => bail!("Cannot convert to UNIX time {} to DateTime<{}>", self.seconds(), offset)
        }
    }
}

#[async_trait(?Send)]
pub(crate) trait LibGit2SignatureExtensions {
    /// Tries to disassemble this [Signature][signature] as `(Username, User ID)`.
    ///
    /// This will search the database (hence it requires a [Executor](sqlx::Executor)) for the
    /// email provided by this [Signature][signature] `email()` method.
    ///
    /// If an entry is found, the registered `username` and `user id` from the database will be returned.
    /// If no entry is found, this [Signature][signature]s `name()` and `None` will be returned.
    ///
    /// This method will return an [Error](anyhow::Error) if the lookup in the database fails for whatever reason.
    ///
    /// If this [Signature][signature]'s name is not valid utf-8, `Ghost` will be returned instead for `username`.
    /// This behaviour is subject to change.
    ///
    /// If this [Signature][signature]'s email is not valid utf-8, `None` will be returned instead of an user id.
    ///
    /// # Example
    ///
    /// ```ignore
    /// # let signature = git2::Signature::now("mellowagain", "mellowagain@example.com");
    /// # let mut transaction = todo!("Find out a way to acquire a Transaction for doc tests");
    ///
    /// let (name, user_id) = signature.try_disassemble(&mut transaction)?;
    /// assert_eq!("mellowagain", name);
    /// ```
    ///
    /// [signature]: git2::Signature
    async fn try_disassemble<'e, E: Executor<'e, Database = Postgres>>(&self, executor: E) -> Result<(String, Option<i32>)>;
}

#[async_trait(?Send)]
impl LibGit2SignatureExtensions for LibGit2Signature<'_> {
    async fn try_disassemble<'e, E: Executor<'e, Database = Postgres>>(&self, executor: E) -> Result<(String, Option<i32>)> {
        let option: Option<(String, i32)> = if let Some(email) = self.email() {
            sqlx::query_as("select username, id from users where lower(email) = lower($1)")
                .bind(email)
                .fetch_optional(executor)
                .await?
        } else {
            None
        };

        Ok(option.map_or_else(
            || (self.name().unwrap_or("Ghost").to_owned(), None),
            |(username, id)| (username, Some(id))
        ))
    }
}

pub(crate) trait GitoxideSignatureExtensions {
    /// Returns the default signature for GitArena.
    /// This is at the moment hardcoded but is subject to change in the future.
    fn gitarena_default() -> GitoxideSignature;
}

impl GitoxideSignatureExtensions for GitoxideSignature {
    fn gitarena_default() -> GitoxideSignature {
        let now = Utc::now();
        let naive = now.naive_utc();

        GitoxideSignature {
            name: BString::from("GitArena"), // TODO: Allow administrators to edit this
            email: BString::from("git@gitarena.com"), // as well as this
            time: GitoxideTime {
                time: naive.timestamp() as u32,
                offset: 0,
                sign: Sign::Plus
            }
        }
    }
}
