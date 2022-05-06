// Copyright 2018 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use crate::{transport::{ListenerEvent, Transport, TransportError}, connection::ListenerId};
use futures::prelude::*;
use multiaddr::Multiaddr;
use std::{error::Error, fmt, io, pin::Pin, sync::Arc, task::{Context, Poll}};

/// Creates a new [`Boxed`] transport from the given transport.
pub fn boxed<T>(transport: T) -> Boxed<T::Output>
where
    T: Transport + Send + Sync + 'static,
    T::Error: Send + Sync,
    T::Dial: Send + 'static,
    T::ListenerUpgrade: Send + 'static,
{
    Boxed {
        inner: Box::new(transport),
    }
}

/// A `Boxed` transport is a `Transport` whose `Dial`, `Listener`
/// and `ListenerUpgrade` futures are `Box`ed and only the `Output`
/// and `Error` types are captured in type variables.
#[pin_project::pin_project]
pub struct Boxed<O> {
    #[pin]
    inner: Box<dyn Abstract<O> + Send + Sync>,
}

type Dial<O> = Pin<Box<dyn Future<Output = io::Result<O>> + Send>>;
type ListenerUpgrade<O> = Pin<Box<dyn Future<Output = io::Result<O>> + Send>>;


trait Abstract<O>{
    fn listen_on(&mut self, listener_id: ListenerId, addr: Multiaddr) -> Result<(), TransportError<io::Error>>;
    fn dial(&mut self, addr: Multiaddr) -> Result<Dial<O>, TransportError<io::Error>>;
    fn dial_as_listener(&mut self, addr: Multiaddr) -> Result<Dial<O>, TransportError<io::Error>>;
    fn address_translation(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr>;
    fn poll_listener(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ListenerEvent<ListenerUpgrade<O>, io::Error>>>;
}

impl<T, O> Abstract<O> for T
where
    T: Transport<Output = O> + 'static,
    T::Error: Send + Sync,
    T::Dial: Send + 'static,
    T::ListenerUpgrade: Send + 'static,
    O: 'static
{
    fn listen_on(&mut self, listener_id: ListenerId,addr: Multiaddr) -> Result<(), TransportError<io::Error>> {
        self.listen_on(listener_id, addr).map_err(|e| e.map(box_err))
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<Dial<O>, TransportError<io::Error>> {
        let fut = self.dial(addr)
            .map(|r| r.map_err(box_err))
            .map_err(|e| e.map(box_err))?;
        Ok(Box::pin(fut) as Dial<_>)
    }

    fn dial_as_listener(&mut self, addr: Multiaddr) -> Result<Dial<O>, TransportError<io::Error>> {
        let fut = self.dial_as_listener(addr)
            .map(|r| r.map_err(box_err))
            .map_err(|e| e.map(box_err))?;
        Ok(Box::pin(fut) as Dial<_>)
    }

    fn address_translation(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.address_translation(server, observed)
    }

    fn poll_listener(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ListenerEvent<ListenerUpgrade<O>, io::Error>>>{
        match self.poll_next_unpin(cx) {
            Poll::Ready(Some(event)) => {
                let event = match event {
                    ListenerEvent::AddressExpired { listener_id, address } => 
                    ListenerEvent::AddressExpired { listener_id, address },
                    ListenerEvent::Closed { listener_id, addresses, reason } => 
                    ListenerEvent::Closed { listener_id, addresses, reason: reason.map_err(box_err) },
                    ListenerEvent::Error { listener_id, error } =>
                    ListenerEvent::Error { listener_id, error: box_err(error) },
                    ListenerEvent::NewAddress { listener_id, address } => 
                    ListenerEvent::NewAddress { listener_id, address },
                    ListenerEvent::Upgrade { listener_id, upgrade, local_addr, remote_addr } => {
                            let up = upgrade.map_err(box_err);
                        ListenerEvent::Upgrade { listener_id, upgrade: Box::pin(up) as ListenerUpgrade<O>, local_addr, remote_addr }
                    }
                };
                Poll::Ready(Some(event))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending
        }
    }
}

impl<O> fmt::Debug for Boxed<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BoxedTransport")
    }
}


impl<O: 'static> Stream for Boxed<O> {
    type Item = ListenerEvent<ListenerUpgrade<O>, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.poll_listener(cx)
    }
}

impl<O: 'static> Transport for Boxed<O> {
    type Output = O;
    type Error = io::Error;
    type ListenerUpgrade = ListenerUpgrade<O>;
    type Dial = Dial<O>;

    fn listen_on(&mut self, listener_id: ListenerId, addr: Multiaddr) -> Result<(), TransportError<Self::Error>> {
        self.inner.listen_on(listener_id, addr)
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        self.inner.dial(addr)
    }

    fn dial_as_listener(&mut self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        self.inner.dial_as_listener(addr)
    }

    fn address_translation(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.address_translation(server, observed)
    }
}

fn box_err<E: Error + Send + Sync + 'static>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}
