pub trait Moves: Send + Sync {
    fn keep(&self, v: Value, name: &str, m: Map) -> Result<i64, ProviderError>;
    fn take(&self, l: List) -> i64;
}
/// The kind's function table: a header, then one slot per method,
/// in declaration order.
///
/// A `guatiao_value *`, `guatiao_map *` or `guatiao_list *`
/// argument that is not `const` is a tree the caller hands
/// over with the call: the slot owns it from then on, on
/// every path, and frees or keeps it; the caller never frees
/// it. It lives in the caller's allocators.
#[repr(C)]
#[doc(hidden)]
#[allow(non_snake_case)]
pub struct MovesVtable {
    pub header: ::guatiao::library::KindHeader,
    pub keep: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            v: *mut ::guatiao::Value,
            name: ::guatiao::Str,
            m: *mut ::guatiao::Map,
            out: *mut i64,
            err: *mut ::guatiao::library::ProviderError,
        ) -> ::guatiao::Status,
    >,
    pub take: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            l: *mut ::guatiao::List,
            out: *mut i64,
        ) -> ::guatiao::Status,
    >,
}
#[allow(non_snake_case, clippy::missing_safety_doc)]
impl MovesVtable {
    /// The table for one implementation of the kind.
    #[doc(hidden)]
    pub const fn of<__T: Moves>() -> Self {
        Self {
            header: ::guatiao::library::KindHeader::new(
                ::core::mem::size_of::<Self>(),
                <dyn Moves as ::guatiao::library::Kind>::FLOOR_HASH,
            ),
            keep: ::core::option::Option::Some(Self::__guatiao_shim_keep::<__T>),
            take: ::core::option::Option::Some(Self::__guatiao_shim_take::<__T>),
        }
    }
    /// One past the last required slot. Frozen.
    #[doc(hidden)]
    pub const fn floor() -> usize {
        Self::take_end()
    }
    /// FNV-1a over the shape and the required signatures, as a
    /// literal, so a C header generator can render it: what a C
    /// implementation writes into the table's header.
    pub const FLOOR_HASH: u32 = 2716657938;
    /// The kind as data, compact JSON: its name, floor hash, and each
    /// slot in table order with its arguments and answer, as the
    /// header declares them, and whether it is required and fallible.
    /// What a binding loads instead of declaring the kind again.
    pub fn description() -> ::guatiao::__private::alloc::string::String {
        let mut out = ::guatiao::__private::alloc::string::String::new();
        out.push_str(
            "{\"name\":\"moves\",\"floor_hash\":2716657938,\"object\":false,\"slots\":[{\"name\":\"keep\",\"required\":true,\"fallible\":true,\"args\":[{\"name\":\"v\",\"type\":\"value\",\"pass\":\"taken\"},{\"name\":\"name\",\"type\":\"str\"},{\"name\":\"m\",\"type\":\"map\",\"pass\":\"taken\"}],\"answer\":{\"type\":\"i64\"}},{\"name\":\"take\",\"required\":true,\"fallible\":false,\"args\":[{\"name\":\"l\",\"type\":\"list\",\"pass\":\"taken\"}],\"answer\":{\"type\":\"i64\"}}]}",
        );
        out
    }
    #[doc(hidden)]
    pub const fn keep_end() -> usize {
        ::core::mem::offset_of!(Self, keep) + ::core::mem::size_of::<usize>()
    }
    #[doc(hidden)]
    pub const fn take_end() -> usize {
        ::core::mem::offset_of!(Self, take) + ::core::mem::size_of::<usize>()
    }
    /// # Safety
    ///
    /// Called through the table `of::<__T>()` built, with the `ctx`
    /// that table's provider declared: a `&__T`.
    #[doc(hidden)]
    pub unsafe extern "C" fn __guatiao_shim_keep<__T: Moves>(
        __ctx: *mut ::core::ffi::c_void,
        v: *mut ::guatiao::Value,
        name: ::guatiao::Str,
        m: *mut ::guatiao::Map,
        __out: *mut i64,
        __err: *mut ::guatiao::library::ProviderError,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let v = unsafe { ::guatiao::library::kind::value_owned(v) };
            let m = unsafe { ::guatiao::library::kind::map_owned(m) };
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(__ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
            let v = match v {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__s) => {
                    return unsafe {
                        ::guatiao::library::kind::write_err(__err, __s.into())
                    };
                }
            };
            let name = match ::core::str::from_utf8(name.into()) {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__e) => {
                    return unsafe {
                        ::guatiao::library::kind::write_err(
                            __err,
                            ::guatiao::Status::from(__e).into(),
                        )
                    };
                }
            };
            let m = match m {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__s) => {
                    return unsafe {
                        ::guatiao::library::kind::write_err(__err, __s.into())
                    };
                }
            };
            match __this.keep(v, name, m) {
                ::core::result::Result::Ok(__answer) => {
                    unsafe { ::guatiao::library::kind::write_out(__out, __answer) }
                }
                ::core::result::Result::Err(__e) => {
                    unsafe { ::guatiao::library::kind::write_err(__err, __e) }
                }
            }
        })
    }
    /// # Safety
    ///
    /// Called through the table `of::<__T>()` built, with the `ctx`
    /// that table's provider declared: a `&__T`.
    #[doc(hidden)]
    pub unsafe extern "C" fn __guatiao_shim_take<__T: Moves>(
        __ctx: *mut ::core::ffi::c_void,
        l: *mut ::guatiao::List,
        __out: *mut i64,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let l = unsafe { ::guatiao::library::kind::list_owned(l) };
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(__ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
            let l = match l {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__s) => {
                    return __s;
                }
            };
            unsafe { ::guatiao::library::kind::write_out(__out, __this.take(l)) }
        })
    }
}
impl ::guatiao::library::Kind for dyn Moves {
    const NAME: &'static str = "moves";
    type Vtable = MovesVtable;
    const FLOOR: usize = MovesVtable::floor();
    const FLOOR_HASH: u32 = MovesVtable::FLOOR_HASH;
    const REQUIRED: &'static [(&'static str, usize)] = &[
        ("keep", MovesVtable::keep_end()),
        ("take", MovesVtable::take_end()),
    ];
    const OBJECT: bool = false;
    fn live_objects(
        seen: &mut ::guatiao::__private::alloc::vec::Vec<&'static str>,
    ) -> usize {
        if seen.contains(&"moves") {
            return 0;
        }
        seen.push("moves");
        0usize
    }
    fn describe(
        seen: &mut ::guatiao::__private::alloc::vec::Vec<&'static str>,
        out: &mut ::guatiao::__private::alloc::vec::Vec<
            ::guatiao::__private::alloc::string::String,
        >,
    ) {
        if seen.contains(&"moves") {
            return;
        }
        seen.push("moves");
        out.push(MovesVtable::description());
    }
    fn as_dyn(remote: &::guatiao::library::Remote<Self>) -> &Self {
        remote
    }
    fn as_dyn_mut(remote: &mut ::guatiao::library::Remote<Self>) -> &mut Self {
        remote
    }
    fn boxed(
        remote: ::guatiao::library::Remote<Self>,
    ) -> ::guatiao::__private::alloc::boxed::Box<Self> {
        ::guatiao::__private::alloc::boxed::Box::new(remote)
    }
    fn shared(
        remote: ::guatiao::library::Remote<Self>,
    ) -> ::guatiao::__private::alloc::sync::Arc<Self> {
        ::guatiao::__private::alloc::sync::Arc::new(remote)
    }
}
unsafe impl ::guatiao::library::kind::Lend for dyn Moves {}
#[allow(clippy::needless_question_mark, clippy::redundant_closure_call)]
impl Moves for ::guatiao::library::Remote<dyn Moves> {
    fn keep(&self, v: Value, name: &str, m: Map) -> Result<i64, ProviderError> {
        let __f: unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            v: *mut ::guatiao::Value,
            name: ::guatiao::Str,
            m: *mut ::guatiao::Map,
            out: *mut i64,
            err: *mut ::guatiao::library::ProviderError,
        ) -> ::guatiao::Status = unsafe {
            self.slot(
                ::core::mem::offset_of!(MovesVtable, keep),
                MovesVtable::keep_end(),
            )
        }
            .expect("validated: a required slot is present and non-null");
        let __arg_name = ::guatiao::Str::new(name);
        let mut __own_v = ::core::mem::ManuallyDrop::new(v);
        let __arg_v = &mut *__own_v as *mut ::guatiao::Value;
        let mut __own_m = ::core::mem::ManuallyDrop::new(m);
        let __arg_m = &mut *__own_m as *mut ::guatiao::Map;
        let mut __out: i64 = ::core::default::Default::default();
        let mut __err = ::guatiao::library::ProviderError::none();
        let __status = unsafe {
            __f(self.ctx(), __arg_v, __arg_name, __arg_m, &mut __out, &mut __err)
        };
        if __status != ::guatiao::Status::GUATIAO_OK {
            return ::core::result::Result::Err(
                ::guatiao::library::kind::take_err(__err, __status),
            );
        }
        ::core::result::Result::Ok(__out)
    }
    fn take(&self, l: List) -> i64 {
        let __f: unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            l: *mut ::guatiao::List,
            out: *mut i64,
        ) -> ::guatiao::Status = unsafe {
            self.slot(
                ::core::mem::offset_of!(MovesVtable, take),
                MovesVtable::take_end(),
            )
        }
            .expect("validated: a required slot is present and non-null");
        let mut __own_l = ::core::mem::ManuallyDrop::new(l);
        let __arg_l = &mut *__own_l as *mut ::guatiao::List;
        let mut __out: i64 = ::core::default::Default::default();
        let __status = unsafe { __f(self.ctx(), __arg_l, &mut __out) };
        if __status != ::guatiao::Status::GUATIAO_OK {
            ::core::panic!(
                "guatiao: the provider failed `{}`, which cannot fail: {}", "take",
                ::core::format_args!("{:?}", __status)
            );
        }
        __out
    }
}
impl ::core::convert::From<::guatiao::library::Remote<dyn Moves>>
for ::guatiao::__private::alloc::boxed::Box<dyn Moves> {
    fn from(remote: ::guatiao::library::Remote<dyn Moves>) -> Self {
        ::guatiao::__private::alloc::boxed::Box::new(remote)
    }
}
impl Moves for ::guatiao::library::Instance<dyn Moves> {
    fn keep(&self, v: Value, name: &str, m: Map) -> Result<i64, ProviderError> {
        (**self).keep(v, name, m)
    }
    fn take(&self, l: List) -> i64 {
        (**self).take(l)
    }
}
impl ::core::convert::From<::guatiao::library::Instance<dyn Moves>>
for ::guatiao::__private::alloc::boxed::Box<dyn Moves> {
    fn from(instance: ::guatiao::library::Instance<dyn Moves>) -> Self {
        ::guatiao::__private::alloc::boxed::Box::new(instance)
    }
}
