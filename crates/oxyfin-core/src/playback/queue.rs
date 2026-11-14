//! Previous and next episode lookup.

use oxyfin_api::ItemId;

use crate::models::Episode;

/// Returns the episodes before and after `current`, in the order of `list`.
pub(crate) fn neighbours(list: &[Episode], current: &ItemId) -> (Option<Episode>, Option<Episode>) {
    let Some(position) = list.iter().position(|episode| &episode.id == current) else {
        return (None, None);
    };
    let previous = position.checked_sub(1).and_then(|i| list.get(i)).cloned();
    (previous, list.get(position + 1).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UserState;

    fn episode(id: &str, season: u32, number: u32) -> Episode {
        Episode {
            id: ItemId::from(id),
            series_id: ItemId::from("series"),
            series_name: None,
            season_id: None,
            title: id.to_owned(),
            season_number: Some(season),
            number: Some(number),
            label: None,
            overview: None,
            thumb: None,
            duration: None,
            user: UserState::default(),
        }
    }

    fn list() -> Vec<Episode> {
        vec![
            episode("s1e1", 1, 1),
            episode("s1e2", 1, 2),
            episode("s2e1", 2, 1),
        ]
    }

    #[test]
    fn neighbours_cross_seasons() {
        let (previous, next) = neighbours(&list(), &ItemId::from("s1e2"));
        assert_eq!(previous.map(|e| e.id), Some(ItemId::from("s1e1")));
        assert_eq!(next.map(|e| e.id), Some(ItemId::from("s2e1")));
    }

    #[test]
    fn first_and_last_have_one_neighbour() {
        let (previous, next) = neighbours(&list(), &ItemId::from("s1e1"));
        assert!(previous.is_none());
        assert_eq!(next.map(|e| e.id), Some(ItemId::from("s1e2")));
        let (previous, next) = neighbours(&list(), &ItemId::from("s2e1"));
        assert_eq!(previous.map(|e| e.id), Some(ItemId::from("s1e2")));
        assert!(next.is_none());
        assert_eq!(neighbours(&list(), &ItemId::from("zzz")), (None, None));
    }
}
