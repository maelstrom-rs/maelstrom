use serde::{Deserialize, Serialize};

#[derive(PartialEq, Clone, Debug, Serialize, Deserialize)]
pub struct DisplayNameResponse {
    pub displayname: String,
}
