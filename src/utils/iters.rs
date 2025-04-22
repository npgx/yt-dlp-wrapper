pub(crate) struct RepeatLast<I, E> {
    inner: I,
    last: Option<E>,
}

impl<I, E: Clone> RepeatLast<I, E> {
    pub(crate) fn new(inner: I) -> Self {
        Self { inner, last: None }
    }
}

impl<I> Iterator for RepeatLast<I, <I as Iterator>::Item>
where
    I: Iterator,
    <I as Iterator>::Item: Clone,
{
    type Item = <I as Iterator>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            None => self.last.clone(),
            Some(item) => {
                self.last.replace(item.clone());
                Some(item)
            }
        }
    }

    // just like Cycle Iter
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.inner.size_hint() {
            sz @ (0, Some(0)) => sz,
            (0, _) => (0, None),
            _ => (usize::MAX, None),
        }
    }
}

pub(crate) trait IntoRepeatLast<I, E> {
    fn repeat_last(self) -> RepeatLast<I, E>;
}

impl<I> IntoRepeatLast<I, <I as Iterator>::Item> for I
where
    I: Iterator,
    <I as Iterator>::Item: Clone,
{
    fn repeat_last(self) -> RepeatLast<I, <I as Iterator>::Item> {
        RepeatLast::new(self)
    }
}
