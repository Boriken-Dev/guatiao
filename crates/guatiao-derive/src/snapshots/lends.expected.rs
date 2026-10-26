pub trait Lends: Send + Sync {
    fn check(&self, config: &mut Map) -> Result<bool, ProviderError>;
    fn fill(&self, v: &mut Value, l: &mut List, name: &str) -> i64;
}
/// The kind's function table: a header, then one slot per method,
/// in declaration order.
///
/// A `guatiao_value *`, `guatiao_map *` or `guatiao_list *`
/// argument its slot's comment calls lent is the caller's tree,
/// for the call only: read it, change it, grow it, but never free
/// it or keep the pointer. Build what goes into it through the
/// allocator it records (`alloc`, the map arm's for a value); a
/// Rust caller moves anything built elsewhere there once the call
/// returns.
#[repr(C)]
#[doc(hidden)]
#[allow(non_snake_case)]
pub struct LendsVtable {
    pub header: ::guatiao::library::KindHeader,
    /// Lent for the call, not handed over: `config`.
    pub check: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            config: *mut ::guatiao::Map,
            out: *mut bool,
            err: *mut ::guatiao::library::ProviderError,
        ) -> ::guatiao::Status,
    >,
    /// Lent for the call, not handed over: `v`, `l`.
    pub fill: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            v: *mut ::guatiao::Value,
            l: *mut ::guatiao::List,
            name: ::guatiao::Str,
            out: *mut i64,
        ) -> ::guatiao::Status,
    >,
}
#[allow(non_snake_case, clippy::missing_safety_doc)]
impl LendsVtable {
    /// The table for one implementation of the kind.
    #[doc(hidden)]
    pub const fn of<__T: Lends>() -> Self {
        Self {
            header: ::guatiao::library::KindHeader::new(
                ::core::mem::size_of::<Self>(),
                <dyn Lends as ::guatiao::library::Kind>::FLOOR_HASH,
            ),
            check: ::core::option::Option::Some(Self::__guatiao_shim_check::<__T>),
            fill: ::core::option::Option::Some(Self::__guatiao_shim_fill::<__T>),
        }
    }
    /// The smallest table a reader accepts: every slot.
    #[doc(hidden)]
    pub const fn floor() -> usize {
        ::core::mem::size_of::<Self>()
    }
    /// FNV-1a over the shape and the required signatures, as a
    /// literal, so a C header generator can render it: what a C
    /// implementation writes into the table's header.
    pub const FLOOR_HASH: u32 = 2814562052;
    /// The kind as data, compact JSON: its name, floor hash, and each
    /// slot in table order with its arguments and answer, as the
    /// header declares them, and whether it is required and fallible.
    /// What a binding loads instead of declaring the kind again.
    pub fn description() -> ::guatiao::__private::alloc::string::String {
        let mut out = ::guatiao::__private::alloc::string::String::new();
        out.push_str(
            "{\"name\":\"lends\",\"floor_hash\":2814562052,\"object\":false,\"slots\":[{\"name\":\"check\",\"required\":true,\"fallible\":true,\"args\":[{\"name\":\"config\",\"type\":\"map\",\"pass\":\"lent\"}],\"answer\":{\"type\":\"bool\"}},{\"name\":\"fill\",\"required\":true,\"fallible\":false,\"args\":[{\"name\":\"v\",\"type\":\"value\",\"pass\":\"lent\"},{\"name\":\"l\",\"type\":\"list\",\"pass\":\"lent\"},{\"name\":\"name\",\"type\":\"str\"}],\"answer\":{\"type\":\"i64\"}}]}",
        );
        out
    }
    /// # Safety
    ///
    /// Called through the table `of::<__T>()` built, with the `ctx`
    /// that table's provider declared: a `&__T`.
    #[doc(hidden)]
    pub unsafe extern "C" fn __guatiao_shim_check<__T: Lends>(
        __ctx: *mut ::core::ffi::c_void,
        config: *mut ::guatiao::Map,
        __out: *mut bool,
        __err: *mut ::guatiao::library::ProviderError,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(__ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
            let config = match unsafe { ::guatiao::library::kind::map_mut_arg(config) } {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__s) => {
                    return unsafe {
                        ::guatiao::library::kind::write_err(__err, __s.into())
                    };
                }
            };
            match __this.check(config) {
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
    pub unsafe extern "C" fn __guatiao_shim_fill<__T: Lends>(
        __ctx: *mut ::core::ffi::c_void,
        v: *mut ::guatiao::Value,
        l: *mut ::guatiao::List,
        name: ::guatiao::Str,
        __out: *mut i64,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(__ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
            let v = match unsafe { ::guatiao::library::kind::value_mut_arg(v) } {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__s) => {
                    return __s;
                }
            };
            let l = match unsafe { ::guatiao::library::kind::list_mut_arg(l) } {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__s) => {
                    return __s;
                }
            };
            let name = match ::core::str::from_utf8(name.into()) {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__e) => {
                    return ::guatiao::Status::from(__e);
                }
            };
            unsafe {
                ::guatiao::library::kind::write_out(__out, __this.fill(v, l, name))
            }
        })
    }
}
impl ::guatiao::library::Kind for dyn Lends {
    const NAME: &'static str = "lends";
    type Vtable = LendsVtable;
    const FLOOR: usize = LendsVtable::floor();
    const FLOOR_HASH: u32 = LendsVtable::FLOOR_HASH;
    const REQUIRED: &'static [(&'static str, usize)] = &[
        (
            "check",
            ::core::mem::offset_of!(LendsVtable, check) + ::core::mem::size_of::<usize>(),
        ),
        (
            "fill",
            ::core::mem::offset_of!(LendsVtable, fill) + ::core::mem::size_of::<usize>(),
        ),
    ];
    const OBJECT: bool = false;
    fn live_objects(
        seen: &mut ::guatiao::__private::alloc::vec::Vec<&'static str>,
    ) -> usize {
        if seen.contains(&"lends") {
            return 0;
        }
        seen.push("lends");
        0usize
    }
    fn describe(
        seen: &mut ::guatiao::__private::alloc::vec::Vec<&'static str>,
        out: &mut ::guatiao::__private::alloc::vec::Vec<
            ::guatiao::__private::alloc::string::String,
        >,
    ) {
        if seen.contains(&"lends") {
            return;
        }
        seen.push("lends");
        out.push(LendsVtable::description());
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
unsafe impl ::guatiao::library::kind::Lend for dyn Lends {}
#[allow(clippy::needless_question_mark, clippy::redundant_closure_call)]
impl Lends for ::guatiao::library::Remote<dyn Lends> {
    fn check(&self, config: &mut Map) -> Result<bool, ProviderError> {
        let __f: unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            config: *mut ::guatiao::Map,
            out: *mut bool,
            err: *mut ::guatiao::library::ProviderError,
        ) -> ::guatiao::Status = unsafe {
            self.slot(::core::mem::offset_of!(LendsVtable, check))
        }
            .expect("validated: a required slot is present and non-null");
        let __home_config = ::guatiao::library::kind::lent_home(&*config);
        let __arg_config = &mut *config as *mut ::guatiao::Map;
        let mut __out: bool = ::core::default::Default::default();
        let mut __err = ::guatiao::library::ProviderError::none();
        let __status = unsafe { __f(self.ctx(), __arg_config, &mut __out, &mut __err) };
        let __settled: ::core::result::Result<(), ::guatiao::library::ProviderError> = ::core::result::Result::Ok(())
            .and(::guatiao::library::kind::lent_settle(&mut *config, __home_config));
        if __status != ::guatiao::Status::GUATIAO_OK {
            return ::core::result::Result::Err(
                ::guatiao::library::kind::take_err(__err, __status),
            );
        }
        if let ::core::result::Result::Err(__e) = __settled {
            return ::core::result::Result::Err(__e);
        }
        ::core::result::Result::Ok(__out)
    }
    fn fill(&self, v: &mut Value, l: &mut List, name: &str) -> i64 {
        let __f: unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            v: *mut ::guatiao::Value,
            l: *mut ::guatiao::List,
            name: ::guatiao::Str,
            out: *mut i64,
        ) -> ::guatiao::Status = unsafe {
            self.slot(::core::mem::offset_of!(LendsVtable, fill))
        }
            .expect("validated: a required slot is present and non-null");
        let __home_v = ::guatiao::library::kind::lent_home(&*v);
        let __arg_v = &mut *v as *mut ::guatiao::Value;
        let __home_l = ::guatiao::library::kind::lent_home(&*l);
        let __arg_l = &mut *l as *mut ::guatiao::List;
        let __arg_name = ::guatiao::Str::new(name);
        let mut __out: i64 = ::core::default::Default::default();
        let __status = unsafe {
            __f(self.ctx(), __arg_v, __arg_l, __arg_name, &mut __out)
        };
        let __settled: ::core::result::Result<(), ::guatiao::library::ProviderError> = ::core::result::Result::Ok(())
            .and(::guatiao::library::kind::lent_settle(&mut *v, __home_v))
            .and(::guatiao::library::kind::lent_settle(&mut *l, __home_l));
        if __status != ::guatiao::Status::GUATIAO_OK {
            ::core::panic!(
                "guatiao: the provider failed `{}`, which cannot fail: {}", "fill",
                ::core::format_args!("{:?}", __status)
            );
        }
        if let ::core::result::Result::Err(__e) = __settled {
            ::core::panic!(
                "guatiao: the provider failed `{}`, which cannot fail: {}", "fill", __e
            );
        }
        __out
    }
}
impl ::core::convert::From<::guatiao::library::Remote<dyn Lends>>
for ::guatiao::__private::alloc::boxed::Box<dyn Lends> {
    fn from(remote: ::guatiao::library::Remote<dyn Lends>) -> Self {
        ::guatiao::__private::alloc::boxed::Box::new(remote)
    }
}
impl Lends for ::guatiao::library::Instance<dyn Lends> {
    fn check(&self, config: &mut Map) -> Result<bool, ProviderError> {
        (**self).check(config)
    }
    fn fill(&self, v: &mut Value, l: &mut List, name: &str) -> i64 {
        (**self).fill(v, l, name)
    }
}
impl ::core::convert::From<::guatiao::library::Instance<dyn Lends>>
for ::guatiao::__private::alloc::boxed::Box<dyn Lends> {
    fn from(instance: ::guatiao::library::Instance<dyn Lends>) -> Self {
        ::guatiao::__private::alloc::boxed::Box::new(instance)
    }
}
