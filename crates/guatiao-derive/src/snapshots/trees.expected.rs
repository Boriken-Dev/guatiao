pub trait Trees: Send + Sync {
    fn map(&self, name: &str) -> Result<Map, ProviderError>;
    fn value(&self) -> Value;
    fn list(&self) -> List {
        List::new()
    }
    /// `map`, its answer built through `alloc`.
    ///
    /// What a call through the kind's table runs, naming the
    /// allocator the caller wants the answer in. Build the answer
    /// there, with the `_in` forms, and it crosses without a copy.
    /// The default answers the method itself, which the caller
    /// then copies into `alloc`.
    #[allow(unused_variables)]
    fn map_in(&self, name: &str, alloc: ::guatiao::Alloc) -> Result<Map, ProviderError> {
        self.map(name)
    }
    /// `value`, its answer built through `alloc`.
    ///
    /// What a call through the kind's table runs, naming the
    /// allocator the caller wants the answer in. Build the answer
    /// there, with the `_in` forms, and it crosses without a copy.
    /// The default answers the method itself, which the caller
    /// then copies into `alloc`.
    #[allow(unused_variables)]
    fn value_in(&self, alloc: ::guatiao::Alloc) -> Value {
        self.value()
    }
    /// `list`, its answer built through `alloc`.
    ///
    /// What a call through the kind's table runs, naming the
    /// allocator the caller wants the answer in. Build the answer
    /// there, with the `_in` forms, and it crosses without a copy.
    /// The default answers the method itself, which the caller
    /// then copies into `alloc`.
    #[allow(unused_variables)]
    fn list_in(&self, alloc: ::guatiao::Alloc) -> List {
        self.list()
    }
}
/// The kind's function table: a header, then one slot per method,
/// in declaration order.
///
/// A slot answering a value, a map or a list finds its out
/// slot holding a carrier, which owns nothing: an empty map
/// or list, or an ABSENT value, whose `alloc` (the map
/// arm's, for a value) names the allocator the caller wants
/// the answer in, or is null. Build the answer through it
/// and the caller keeps it as it is; built anywhere else,
/// the caller copies it. Either way, write the answer over
/// the carrier without freeing it.
#[repr(C)]
#[doc(hidden)]
#[allow(non_snake_case)]
pub struct TreesVtable {
    pub header: ::guatiao::library::KindHeader,
    pub map: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            name: ::guatiao::Str,
            out: *mut ::guatiao::Map,
            err: *mut ::guatiao::library::ProviderError,
        ) -> ::guatiao::Status,
    >,
    pub value: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            out: *mut ::guatiao::Value,
        ) -> ::guatiao::Status,
    >,
    pub list: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            out: *mut ::guatiao::List,
        ) -> ::guatiao::Status,
    >,
}
#[allow(non_snake_case, clippy::missing_safety_doc)]
impl TreesVtable {
    /// The table for one implementation of the kind.
    #[doc(hidden)]
    pub const fn of<__T: Trees>() -> Self {
        Self {
            header: ::guatiao::library::KindHeader::new(
                ::core::mem::size_of::<Self>(),
                <dyn Trees as ::guatiao::library::Kind>::FLOOR_HASH,
            ),
            map: ::core::option::Option::Some(Self::__guatiao_shim_map::<__T>),
            value: ::core::option::Option::Some(Self::__guatiao_shim_value::<__T>),
            list: ::core::option::Option::Some(Self::__guatiao_shim_list::<__T>),
        }
    }
    /// One past the last required slot. Frozen.
    #[doc(hidden)]
    pub const fn floor() -> usize {
        Self::value_end()
    }
    /// FNV-1a over the shape and the required signatures, as a
    /// literal, so a C header generator can render it: what a C
    /// implementation writes into the table's header.
    pub const FLOOR_HASH: u32 = 3903901445;
    #[doc(hidden)]
    pub const fn map_end() -> usize {
        ::core::mem::offset_of!(Self, map) + ::core::mem::size_of::<usize>()
    }
    #[doc(hidden)]
    pub const fn value_end() -> usize {
        ::core::mem::offset_of!(Self, value) + ::core::mem::size_of::<usize>()
    }
    #[doc(hidden)]
    pub const fn list_end() -> usize {
        ::core::mem::offset_of!(Self, list) + ::core::mem::size_of::<usize>()
    }
    /// # Safety
    ///
    /// Called through the table `of::<__T>()` built, with the `ctx`
    /// that table's provider declared: a `&__T`.
    #[doc(hidden)]
    pub unsafe extern "C" fn __guatiao_shim_map<__T: Trees>(
        __ctx: *mut ::core::ffi::c_void,
        name: ::guatiao::Str,
        __out: *mut ::guatiao::Map,
        __err: *mut ::guatiao::library::ProviderError,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(__ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
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
            let __alloc = unsafe { ::guatiao::library::kind::return_alloc(__out) };
            match __this.map_in(name, __alloc) {
                ::core::result::Result::Ok(__answer) => {
                    match ::guatiao::library::kind::map_ret(__answer, __alloc) {
                        ::core::result::Result::Ok(__v) => {
                            unsafe { ::guatiao::library::kind::write_out(__out, __v) }
                        }
                        ::core::result::Result::Err(__e) => {
                            unsafe { ::guatiao::library::kind::write_err(__err, __e) }
                        }
                    }
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
    pub unsafe extern "C" fn __guatiao_shim_value<__T: Trees>(
        __ctx: *mut ::core::ffi::c_void,
        __out: *mut ::guatiao::Value,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(__ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
            let __alloc = unsafe { ::guatiao::library::kind::return_alloc(__out) };
            match ::guatiao::library::kind::value_ret(
                __this.value_in(__alloc),
                __alloc,
            ) {
                ::core::result::Result::Ok(__v) => {
                    unsafe { ::guatiao::library::kind::write_out(__out, __v) }
                }
                ::core::result::Result::Err(__e) => {
                    unsafe {
                        ::guatiao::library::kind::write_err(::core::ptr::null_mut(), __e)
                    }
                }
            }
        })
    }
    /// # Safety
    ///
    /// Called through the table `of::<__T>()` built, with the `ctx`
    /// that table's provider declared: a `&__T`.
    #[doc(hidden)]
    pub unsafe extern "C" fn __guatiao_shim_list<__T: Trees>(
        __ctx: *mut ::core::ffi::c_void,
        __out: *mut ::guatiao::List,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(__ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
            let __alloc = unsafe { ::guatiao::library::kind::return_alloc(__out) };
            match ::guatiao::library::kind::list_ret(__this.list_in(__alloc), __alloc) {
                ::core::result::Result::Ok(__v) => {
                    unsafe { ::guatiao::library::kind::write_out(__out, __v) }
                }
                ::core::result::Result::Err(__e) => {
                    unsafe {
                        ::guatiao::library::kind::write_err(::core::ptr::null_mut(), __e)
                    }
                }
            }
        })
    }
}
impl ::guatiao::library::Kind for dyn Trees {
    const NAME: &'static str = "trees";
    type Vtable = TreesVtable;
    const FLOOR: usize = TreesVtable::floor();
    const FLOOR_HASH: u32 = TreesVtable::FLOOR_HASH;
    const REQUIRED: &'static [(&'static str, usize)] = &[
        ("map", TreesVtable::map_end()),
        ("value", TreesVtable::value_end()),
    ];
    const OBJECT: bool = false;
    fn live_objects(
        seen: &mut ::guatiao::__private::alloc::vec::Vec<&'static str>,
    ) -> usize {
        if seen.contains(&"trees") {
            return 0;
        }
        seen.push("trees");
        0usize
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
unsafe impl ::guatiao::library::kind::Lend for dyn Trees {}
#[allow(clippy::needless_question_mark, clippy::redundant_closure_call)]
impl Trees for ::guatiao::library::Remote<dyn Trees> {
    fn map_in(&self, name: &str, alloc: ::guatiao::Alloc) -> Result<Map, ProviderError> {
        let __alloc = alloc;
        let __f: unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            name: ::guatiao::Str,
            out: *mut ::guatiao::Map,
            err: *mut ::guatiao::library::ProviderError,
        ) -> ::guatiao::Status = unsafe {
            self.slot(::core::mem::offset_of!(TreesVtable, map), TreesVtable::map_end())
        }
            .expect("validated: a required slot is present and non-null");
        let __arg_name = ::guatiao::Str::new(name);
        let mut __out = <::guatiao::Map as ::guatiao::library::kind::Carrier>::carrier(
            __alloc,
        );
        let mut __err = ::guatiao::library::ProviderError::none();
        let __status = unsafe { __f(self.ctx(), __arg_name, &mut __out, &mut __err) };
        if __status != ::guatiao::Status::GUATIAO_OK {
            return ::core::result::Result::Err(
                ::guatiao::library::kind::take_err(__err, __status),
            );
        }
        ::core::result::Result::Ok(
            match ::guatiao::library::kind::map_ret(__out, __alloc) {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__e) => {
                    return ::core::result::Result::Err(__e);
                }
            },
        )
    }
    fn map(&self, name: &str) -> Result<Map, ProviderError> {
        self.map_in(name, ::guatiao::Alloc::rust())
    }
    fn value_in(&self, alloc: ::guatiao::Alloc) -> Value {
        let __alloc = alloc;
        let __f: unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            out: *mut ::guatiao::Value,
        ) -> ::guatiao::Status = unsafe {
            self.slot(
                ::core::mem::offset_of!(TreesVtable, value),
                TreesVtable::value_end(),
            )
        }
            .expect("validated: a required slot is present and non-null");
        let mut __out = <::guatiao::Value as ::guatiao::library::kind::Carrier>::carrier(
            __alloc,
        );
        let __status = unsafe { __f(self.ctx(), &mut __out) };
        if __status != ::guatiao::Status::GUATIAO_OK {
            ::core::panic!(
                "guatiao: the provider failed `{}`, which cannot fail: {}", "value",
                ::core::format_args!("{:?}", __status)
            );
        }
        match ::guatiao::library::kind::value_ret(__out, __alloc) {
            ::core::result::Result::Ok(__v) => __v,
            ::core::result::Result::Err(__e) => {
                ::core::panic!(
                    "guatiao: the provider failed `{}`, which cannot fail: {}", "value",
                    __e
                );
            }
        }
    }
    fn value(&self) -> Value {
        self.value_in(::guatiao::Alloc::rust())
    }
    fn list_in(&self, alloc: ::guatiao::Alloc) -> List {
        let __alloc = alloc;
        match unsafe {
            self.slot::<
                    unsafe extern "C" fn(
                        ctx: *mut ::core::ffi::c_void,
                        out: *mut ::guatiao::List,
                    ) -> ::guatiao::Status,
                >(::core::mem::offset_of!(TreesVtable, list), TreesVtable::list_end())
        } {
            ::core::option::Option::Some(__f) => {
                let mut __out = <::guatiao::List as ::guatiao::library::kind::Carrier>::carrier(
                    __alloc,
                );
                let __status = unsafe { __f(self.ctx(), &mut __out) };
                if __status != ::guatiao::Status::GUATIAO_OK {
                    ::core::panic!(
                        "guatiao: the provider failed `{}`, which cannot fail: {}",
                        "list", ::core::format_args!("{:?}", __status)
                    );
                }
                match ::guatiao::library::kind::list_ret(__out, __alloc) {
                    ::core::result::Result::Ok(__v) => __v,
                    ::core::result::Result::Err(__e) => {
                        ::core::panic!(
                            "guatiao: the provider failed `{}`, which cannot fail: {}",
                            "list", __e
                        );
                    }
                }
            }
            ::core::option::Option::None => {
                let __out = (|| -> List { List::new() })();
                match ::guatiao::library::kind::list_ret(__out, __alloc) {
                    ::core::result::Result::Ok(__v) => __v,
                    ::core::result::Result::Err(__e) => {
                        ::core::panic!(
                            "guatiao: the provider failed `{}`, which cannot fail: {}",
                            "list", __e
                        );
                    }
                }
            }
        }
    }
    fn list(&self) -> List {
        self.list_in(::guatiao::Alloc::rust())
    }
}
impl ::core::convert::From<::guatiao::library::Remote<dyn Trees>>
for ::guatiao::__private::alloc::boxed::Box<dyn Trees> {
    fn from(remote: ::guatiao::library::Remote<dyn Trees>) -> Self {
        ::guatiao::__private::alloc::boxed::Box::new(remote)
    }
}
impl Trees for ::guatiao::library::Instance<dyn Trees> {
    fn map(&self, name: &str) -> Result<Map, ProviderError> {
        (**self).map(name)
    }
    fn map_in(&self, name: &str, alloc: ::guatiao::Alloc) -> Result<Map, ProviderError> {
        (**self).map_in(name, alloc)
    }
    fn value(&self) -> Value {
        (**self).value()
    }
    fn value_in(&self, alloc: ::guatiao::Alloc) -> Value {
        (**self).value_in(alloc)
    }
    fn list(&self) -> List {
        (**self).list()
    }
    fn list_in(&self, alloc: ::guatiao::Alloc) -> List {
        (**self).list_in(alloc)
    }
}
impl ::core::convert::From<::guatiao::library::Instance<dyn Trees>>
for ::guatiao::__private::alloc::boxed::Box<dyn Trees> {
    fn from(instance: ::guatiao::library::Instance<dyn Trees>) -> Self {
        ::guatiao::__private::alloc::boxed::Box::new(instance)
    }
}
