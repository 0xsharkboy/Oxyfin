//! Media segments of an item.

use crate::models::{MediaSegmentDto, QueryResult};
use crate::{Error, ItemId, Result, Session};

impl Session {
    /// Fetches the segments of an item (`GET MediaSegments/{id}`), in server order.
    ///
    /// A `404 Not Found` gives an empty list: servers answer it for an item
    /// they do not know, and servers older than 10.10 lack the route.
    pub async fn media_segments(&self, id: &ItemId) -> Result<Vec<MediaSegmentDto>> {
        let result: Result<QueryResult<MediaSegmentDto>> = self
            .server()
            .http()
            .get(Some(self.token()), &format!("MediaSegments/{id}"), &[])
            .await;
        match result {
            Ok(page) => Ok(page.items),
            Err(Error::Status { status: 404, .. }) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }
}
