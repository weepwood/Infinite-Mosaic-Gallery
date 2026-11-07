//! Bounded GPUI decoded-image cache, based on the Apache-2.0 GPUI Kit documentation.
//! Recycles GPU texture resources when entries are evicted or the view is destroyed.
use std::{collections::VecDeque, sync::Arc};
use gpui_kit::*;

pub struct RecentImageCache {
    capacity: usize,
    items: VecDeque<(Resource, ImageCacheItem)>,
}

impl RecentImageCache {
    pub fn new(capacity: usize, cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self {
            capacity: capacity.max(1),
            items: VecDeque::new(),
        });
        cx.observe_release(&cache, |cache, cx| {
            for (_, item) in cache.items.drain(..) {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
        }).detach();
        cache
    }
}

impl ImageCache for RecentImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let entry = match self.items.iter().position(|(stored, _)| stored == resource) {
            Some(index) => self.items.remove(index).expect("existing cache entry"),
            None => (resource.clone(), ImageCacheItem::new(resource, cx)),
        };
        self.items.push_front(entry);
        while self.items.len() > self.capacity {
            if let Some((_, item)) = self.items.pop_back() {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, Some(window));
                }
            }
        }
        self.items.front().and_then(|(_, item)| item.use_image(window))
    }
}
