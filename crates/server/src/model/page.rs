use serde::{Deserialize, Serialize};

/// used for page request
#[derive(Serialize, Deserialize, utoipa::ToSchema, Debug, Clone, Copy)]
pub struct Page {
    /// Current page number, Default is 1
    #[serde(default = "default_page_no")]
    page: i32,
    /// Number of items per page, Default is 20
    #[serde(default = "default_page_size")]
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
