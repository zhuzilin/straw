//! Actual finite visibility retry loop, preserving failures and the shared budget.
#[cfg(straw_verus)]
use vstd::prelude::*;
#[cfg_attr(straw_verus, verus_verify)]
pub enum Failure<E> {
    Visibility(E),
    Permanent(E),
}
#[cfg_attr(straw_verus, verus_verify)]
pub enum Error<E> {
    Exhausted(E),
    Permanent(E),
}
#[cfg(straw_verus)]
verus! {
    pub trait Model {
        type Output;type Error;type Loan;
        #[verifier::prophetic]
        spec fn loan(&self)->Self::Loan;
        spec fn valid(&self)->bool;
        spec fn accepts(&self,value:Self::Output)->bool;
        spec fn attempts(&self)->Seq<Result<Self::Output,Failure<Self::Error>>>;
    }
    pub trait ObserverModel:View<V=Seq<u64>> {}
    pub open spec fn transient<T,E>(calls:Seq<Result<T,Failure<E>>>)->bool {
        forall|i:int| 0<=i<calls.len() ==> calls[i] is Err && calls[i]->Err_0 is Visibility
    }
    pub open spec fn outcome<T,E>(last:Result<T,Failure<E>>,result:Result<T,Error<E>>,full:bool)->bool {
        match last {
            Ok(value)=>result==Ok(value),
            Err(Failure::Permanent(error))=>result==Err(Error::Permanent(error)),
            Err(Failure::Visibility(error))=>full && result==Err(Error::Exhausted(error)),
        }
    }
    pub open spec fn receipt<T,E>(calls:Seq<Result<T,Failure<E>>>,delays:Seq<u64>,waits:Seq<u64>,result:Result<T,Error<E>>)->bool {
        0<calls.len()<=delays.len()+1 && transient(calls.drop_last())
            && waits==delays.take(calls.len() as int-1)
            && outcome(calls.last(),result,calls.len()==delays.len()+1)
    }
}
#[cfg(not(straw_verus))]
pub trait Model {
    type Output;
    type Error;
}
#[cfg(not(straw_verus))]
pub trait ObserverModel {}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Attempt: Model {
    #[cfg_attr(straw_verus,verus_spec(result=>requires old(self).valid(),ensures final(self).valid(),final(self).loan()==old(self).loan(),final(self).attempts()==old(self).attempts().push(result),result.is_ok() ==> final(self).accepts(result->Ok_0),))]
    fn read(&mut self) -> Result<Self::Output, Failure<Self::Error>>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Observer<E>: ObserverModel {
    #[cfg_attr(straw_verus,verus_spec(ensures final(self)@==old(self)@,))]
    fn recovered(&mut self, retries: usize);
    #[cfg_attr(straw_verus,verus_spec(ensures final(self)@==old(self)@.push(delay),))]
    fn wait(&mut self, retry: usize, limit: usize, delay: u64, error: &E);
}
#[cfg_attr(straw_verus,verus_spec(result=>requires old(source).valid(),
    ensures final(source).valid(),final(source).loan()==old(source).loan(),result.is_ok() ==> final(source).accepts(result->Ok_0),
        old(source).attempts().is_prefix_of(final(source).attempts()),old(observer)@.is_prefix_of(final(observer)@),
        receipt(final(source).attempts().skip(old(source).attempts().len() as int),delays@,final(observer)@.skip(old(observer)@.len() as int),result),
))]
pub fn run<A: Attempt, O: Observer<A::Error>>(
    source: &mut A,
    observer: &mut O,
    delays: &[u64],
) -> Result<A::Output, Error<A::Error>> {
    let mut retry = 0usize;
    #[cfg_attr(straw_verus,verus_spec(invariant retry<=delays.len(),source.valid(),source.loan()==old(source).loan(),
        source.attempts().len()==old(source).attempts().len()+retry,
        old(source).attempts().is_prefix_of(source.attempts()),
        transient(source.attempts().skip(old(source).attempts().len() as int)),
        observer@==old(observer)@+delays@.take(retry as int),
        decreases delays.len()-retry,
    ))]
    loop {
        #[cfg(straw_verus)]
        let before = verus_exec_expr!(Ghost(source.attempts()));
        let outcome = source.read();
        #[cfg(straw_verus)]
        proof! {
            assert(source.attempts().skip(old(source).attempts().len() as int)==before@.skip(old(source).attempts().len() as int).push(outcome));
            assert((old(observer)@+delays@.take(retry as int)).skip(old(observer)@.len() as int)==delays@.take(retry as int));
        }
        match outcome {
            Ok(value) => {
                if retry > 0 {
                    observer.recovered(retry);
                }
                return Ok(value);
            }
            Err(Failure::Permanent(error)) => return Err(Error::Permanent(error)),
            Err(Failure::Visibility(error)) => {
                let Some(delay) = delays.get(retry) else {
                    return Err(Error::Exhausted(error));
                };
                observer.wait(retry + 1, delays.len(), *delay, &error);
                #[cfg(straw_verus)]
                proof! {assert(delays@.take(retry as int+1)==delays@.take(retry as int).push(*delay));}
                retry += 1;
            }
        }
    }
}
#[cfg_attr(straw_verus,verus_spec(result=>ensures result==(kind@=="ExtentVisibility"@),))]
pub fn visibility(kind: &str) -> bool {
    kind == "ExtentVisibility"
}
#[cfg_attr(straw_verus,verus_spec(ensures final(kind)@=="CorruptData"@,))]
pub fn exhausted(kind: &mut String) {
    *kind = "CorruptData".to_owned();
}

#[cfg_attr(straw_verus, verus_verify)]
pub trait Log<E> {
    fn recovered(&mut self, retries: usize);
    fn retry(&mut self, retry: usize, limit: usize, delay: u64, error: &E);
}
#[cfg(straw_verus)]
verus! {pub trait WaitModel:View<V=Seq<u64>> {}}
#[cfg(not(straw_verus))]
pub trait WaitModel {}
#[cfg(not(straw_verus))]
impl<T> WaitModel for T {}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Wait: WaitModel {
    #[cfg_attr(straw_verus,verus_spec(ensures final(self)@==old(self)@.push(delay),))]
    fn wait(&mut self, delay: u64);
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Observe<L, W: Wait> {
    log: L,
    wait: W,
}
#[cfg(straw_verus)]
verus! {impl<L,W:Wait> View for Observe<L,W> {type V=Seq<u64>;closed spec fn view(&self)->Self::V {self.wait@}}}
#[cfg_attr(straw_verus, verus_verify)]
impl<L, W: Wait> ObserverModel for Observe<L, W> {}
#[cfg_attr(straw_verus, verus_verify)]
impl<L, W: Wait> Observe<L, W> {
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result@==wait@,))]
    pub fn new(log: L, wait: W) -> Self {
        Self { log, wait }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<E, L: Log<E>, W: Wait> Observer<E> for Observe<L, W> {
    fn recovered(&mut self, retries: usize) {
        self.log.recovered(retries);
    }
    fn wait(&mut self, retry: usize, limit: usize, delay: u64, error: &E) {
        self.log.retry(retry, limit, delay, error);
        self.wait.wait(delay);
    }
}

// A single arbitrary attempt. This adapter proves result/budget correspondence;
// it grants no business validity to returned payloads. Typed callers such as
// record_index::Attempt establish their stronger cache/authentication invariant.
#[cfg(straw_verus)]
verus! {
    pub trait CallableModel {
        type Output;type Error;
        spec fn calls(&self)->Seq<Result<Self::Output,Failure<Self::Error>>>;
    }
}
#[cfg(not(straw_verus))]
pub trait CallableModel {
    type Output;
    type Error;
}
#[cfg_attr(straw_verus, verus_verify)]
pub trait Callable: CallableModel {
    #[cfg_attr(straw_verus,verus_spec(result=>ensures final(self).calls()==old(self).calls().push(result),))]
    fn read(&mut self) -> Result<Self::Output, Failure<Self::Error>>;
}
#[cfg_attr(straw_verus, verus_verify)]
pub struct Calls<C: Callable> {
    call: C,
}
#[cfg(straw_verus)]
verus! {
    impl<C:Callable> Model for Calls<C> {
        type Output=C::Output;type Error=C::Error;type Loan=();
        #[verifier::prophetic] closed spec fn loan(&self)->() {()}
        closed spec fn valid(&self)->bool {true}
        closed spec fn accepts(&self,value:Self::Output)->bool {true}
        closed spec fn attempts(&self)->Seq<Result<Self::Output,Failure<Self::Error>>> {self.call.calls()}
    }
}
#[cfg(not(straw_verus))]
impl<C: Callable> Model for Calls<C> {
    type Output = C::Output;
    type Error = C::Error;
}
#[cfg_attr(straw_verus, verus_verify)]
impl<C: Callable> Calls<C> {
    #[cfg_attr(straw_verus,verus_spec(result=>ensures result.valid(),result.attempts()==call.calls(),))]
    pub fn new(call: C) -> Self {
        Self { call }
    }
}
#[cfg_attr(straw_verus, verus_verify)]
impl<C: Callable> Attempt for Calls<C> {
    fn read(&mut self) -> Result<Self::Output, Failure<Self::Error>> {
        self.call.read()
    }
}
