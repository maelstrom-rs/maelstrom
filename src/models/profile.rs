use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisplayNameResponse {
    pub displayname: String,
}
