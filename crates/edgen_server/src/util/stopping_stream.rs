/* Copyright 2023- The Binedge, Lda team. All rights reserved.
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *     http://www.apache.org/licenses/LICENSE-2.0
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
#[allow(unused_imports)] // to avoid the warning on a trait we need (a compiler glitch)
use futures::StreamExt;

/// A [`Stream`] that collects a sequence of [`String`] chunks, and re-emits them when it is
/// impossible for those chunks to be one or more stop words.
#[pin_project::pin_project]
pub struct StoppingStream<T> {
    /// The inner stream.
    #[pin]
    inner: T,

    /// The stop words (phrases) that this stream should stop at.
    ///
    /// These are never emitted downstream, and the stream will yield with `Pending` until it
    /// is impossible for any stop word to be generated.
    stop_words: Vec<String>,

    /// If this stream is uncertain whether it's collecting a stop word, this buffer contains
    /// the working contents of the stream so far.
    working_buf: String,

    /// If `true`, the inner stream has completed, and this wrapper shouldn't yield any more values.
    is_fused: bool,
}

impl<T> StoppingStream<T>
where
    T: Stream<Item = String>,
{
    /// Creates a new stopping stream from the given base stream and a collection of stop words.
    ///
    /// The stop words are never emitted, and the stream will yield `None` when a stop word is
    /// generated by the inner stream.
    pub fn wrap_with_stop_words(inner: T, stop_words: impl Into<Vec<String>>) -> Self {
        Self {
            inner,
            stop_words: stop_words.into(),
            working_buf: String::new(),
            is_fused: false,
        }
    }
}

impl<T> Stream for StoppingStream<T>
where
    T: Stream<Item = String> + Unpin,
{
    type Item = String;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        if *this.is_fused {
            return Poll::Ready(None);
        }

        loop {
            let token = match this.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(token)) => token,
                Poll::Ready(None) => {
                    self.is_fused = true;

                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            };

            this.working_buf.push_str(&token);

            let mut should_emit = true;

            'stop_words: for stop_word in &*this.stop_words {
                if this.working_buf.starts_with(stop_word) {
                    return Poll::Ready(None);
                }

                if stop_word.starts_with(&*this.working_buf) {
                    // We may currently be generating this stop word.
                    //
                    // Stall emission of the working buffer until we can be sure that we're not.
                    should_emit = false;

                    break 'stop_words;
                }
            }

            if should_emit {
                let out_buf = std::mem::take(this.working_buf);

                return Poll::Ready(Some(out_buf));
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn stopping_stream_middle() {
        let stream_content = concat!("apple\n", "banana\n", "coconut\n", "dill\n", "eggplant\n",);

        let content_stream =
            futures::stream::iter(stream_content.lines().map(|line| line.to_string()));

        let stopping_stream = StoppingStream::wrap_with_stop_words(
            content_stream,
            vec!["coconut".to_string(), "eggplant".to_string()],
        );

        assert_eq!(
            stopping_stream.collect::<Vec<_>>().await,
            vec!["apple", "banana"]
        );
    }

    #[tokio::test]
    async fn stopping_stream_start() {
        let stream_content = concat!("apple\n", "banana\n", "coconut\n", "dill\n", "eggplant\n",);

        let content_stream =
            futures::stream::iter(stream_content.lines().map(|line| line.to_string()));

        let stopping_stream = StoppingStream::wrap_with_stop_words(
            content_stream,
            vec!["apple".to_string(), "eggplant".to_string()],
        );

        assert_eq!(
            stopping_stream.collect::<Vec<_>>().await,
            vec![] as Vec<String>,
        );
    }

    #[tokio::test]
    async fn stopping_stream_end() {
        let stream_content = concat!("apple\n", "banana\n", "coconut\n", "dill\n", "eggplant\n",);

        let content_stream =
            futures::stream::iter(stream_content.lines().map(|line| line.to_string()));

        let stopping_stream =
            StoppingStream::wrap_with_stop_words(content_stream, vec!["eggplant".to_string()]);

        assert_eq!(
            stopping_stream.collect::<Vec<_>>().await,
            vec!["apple", "banana", "coconut", "dill"],
        );
    }

    #[tokio::test]
    async fn stopping_stream_all() {
        let stream = concat!("apple\n", "banana\n", "coconut\n", "dill\n", "eggplant\n");

        for i in 0..5 {
            let stream_content = stream.lines().map(|line| line.to_string());
            let v: Vec<String> = stream_content.clone().collect();
            let mut expected: Vec<String> = Vec::with_capacity(i);
            for y in 0..i {
                expected.push(v[y].to_string());
            }

            let content_stream = futures::stream::iter(stream_content);

            let stopping_stream =
                StoppingStream::wrap_with_stop_words(content_stream, vec![v[i].to_string()]);

            println!("expected for {}: {:?}", v[i], expected);

            assert_eq!(stopping_stream.collect::<Vec<_>>().await, expected,);
        }
    }
}
