use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};

/// used for page request
#[derive(Serialize, Deserialize, utoipa::ToSchema, utoipa::IntoParams, Debug, Clone, Copy)]
pub struct Page {
    /// Current page number, Default is 1
    #[serde(
        default = "default_page_no",
        deserialize_with = "crate::model::page::de_i32_from_any"
    )]
    page: i32,
    /// Number of items per page, Default is 20
    #[serde(
        default = "default_page_size",
        deserialize_with = "crate::model::page::de_i32_from_any"
    )]
    page_size: i32,
}

/// used for pagination response
#[derive(Serialize, utoipa::ToSchema, Debug, Deserialize)]
pub struct Pagination {
    /// Current page number
    pub page: i32,
    /// Number of items per page
    pub page_size: i32,
    /// Total number of items
    pub total: u64,
    /// Total number of pages
    pub total_pages: u64,
}

impl Page {
    pub fn offset(&self) -> i32 {
        i32::max(self.page() - 1, 0) * self.page_size()
    }

    pub fn page(&self) -> i32 {
        if self.page > 0 {
            self.page
        } else {
            1 // Default page number if not set or invalid
        }
    }

    pub fn page_size(&self) -> i32 {
        if self.page_size > 0 {
            self.page_size
        } else {
            20 // Default page size if not set or invalid
        }
    }
}

fn default_page_no() -> i32 {
    1 // Default page number
}

fn default_page_size() -> i32 {
    20 // Default page size
}

#[derive(Deserialize)]
#[serde(untagged)]
enum I32OrString {
    I(i64),
    S(String),
}

pub fn de_i32_from_any<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = I32OrString::deserialize(deserializer)?;
    match v {
        I32OrString::I(n) => i32::try_from(n).map_err(|_| D::Error::custom("out of range for i32")),
        I32OrString::S(s) => s
            .trim()
            .parse::<i32>()
            .map_err(|_| D::Error::custom("invalid i32 string")),
    }
}

pub fn de_opt_i32_from_any<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<I32OrString>::deserialize(deserializer)?;
    match v {
        None => Ok(None),
        Some(I32OrString::I(n)) => i32::try_from(n)
            .map(Some)
            .map_err(|_| D::Error::custom("out of range for i32")),
        Some(I32OrString::S(s)) => {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                s.trim()
                    .parse::<i32>()
                    .map(Some)
                    .map_err(|_| D::Error::custom("invalid i32 string"))
            }
        }
    }
}
