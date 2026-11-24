//! Rows of a list updated in place: the rows that stay keep their elements, with their hover,
//! their focus and their open menu.

use std::collections::HashSet;

use slint::{Model, SharedString, VecModel};

use crate::{CardData, EpisodeData};

/// A row of a list that stands for one item.
pub(crate) trait ItemRow: Clone + PartialEq + 'static {
    /// Identifier of the item.
    fn id(&self) -> &SharedString;
}

impl ItemRow for CardData {
    fn id(&self) -> &SharedString {
        &self.id
    }
}

impl ItemRow for EpisodeData {
    fn id(&self) -> &SharedString {
        &self.id
    }
}

/// Whether `rows` stand for the same items as `model`, in the same order.
pub(crate) fn same_items<T: ItemRow>(model: &VecModel<T>, rows: &[T]) -> bool {
    model.row_count() == rows.len()
        && model
            .iter()
            .zip(rows)
            .all(|(old, new)| old.id() == new.id())
}

/// Makes `model` hold `rows`, touching only the rows that differ.
pub(crate) fn update<T: ItemRow>(model: &VecModel<T>, rows: Vec<T>) {
    let wanted: HashSet<&SharedString> = rows.iter().map(ItemRow::id).collect();
    let held: HashSet<SharedString> = model.iter().map(|row| row.id().clone()).collect();
    if wanted.len() != rows.len() || held.len() != model.row_count() {
        // The same item twice: rows cannot be told apart.
        return model.set_vec(rows);
    }
    for index in (0..model.row_count()).rev() {
        if model
            .row_data(index)
            .is_some_and(|row| !wanted.contains(row.id()))
        {
            model.remove(index);
        }
    }
    for (index, new) in rows.into_iter().enumerate() {
        match model.row_data(index) {
            Some(old) if old.id() == new.id() => {
                if old != new {
                    model.set_row_data(index, new);
                }
            }
            _ => {
                let moved = held.contains(new.id()).then(|| {
                    (index..model.row_count()).find(|place| {
                        model
                            .row_data(*place)
                            .is_some_and(|row| row.id() == new.id())
                    })
                });
                if let Some(Some(place)) = moved {
                    model.remove(place);
                }
                model.insert(index, new);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str) -> CardData {
        CardData {
            id: id.into(),
            ..CardData::default()
        }
    }

    fn cards(ids: &str) -> Vec<CardData> {
        ids.chars().map(|id| card(&id.to_string())).collect()
    }

    fn ids(model: &VecModel<CardData>) -> String {
        model.iter().map(|card| card.id.to_string()).collect()
    }

    #[test]
    fn any_list_becomes_any_other() {
        let lists = [
            "", "a", "abc", "cba", "bcd", "xabc", "abcx", "axbc", "aab", "dx",
        ];
        for from in lists {
            for to in lists {
                let model = VecModel::from(cards(from));
                update(&model, cards(to));
                assert_eq!(ids(&model), to, "from {from}");
            }
        }
    }

    #[test]
    fn a_row_takes_its_new_state() {
        let model = VecModel::from(cards("abc"));
        let mut rows = cards("bc");
        rows[0].watched = true;
        update(&model, rows);
        let watched: Vec<bool> = model.iter().map(|card| card.watched).collect();
        assert_eq!(watched, [true, false]);
    }

    #[test]
    fn items_are_the_same_only_in_the_same_order() {
        let model = VecModel::from(cards("abc"));
        assert!(same_items(&model, &cards("abc")));
        assert!(!same_items(&model, &cards("acb")));
        assert!(!same_items(&model, &cards("ab")));
    }
}
