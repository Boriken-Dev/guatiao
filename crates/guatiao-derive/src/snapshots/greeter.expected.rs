pub trait Greeter: Send + Sync {
    fn greet(&self, name: &str) -> Result<String, ProviderError>;
    fn count(&self, bytes: &[u8], flag: bool) -> i64 {
        bytes.len() as i64
    }
}
#[repr(C)]
#[doc(hidden)]
#[allow(non_snake_case)]
pub struct GreeterVtable {
    pub header: ::guatiao::library::KindHeader,
    pub greet: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            name: ::guatiao::Str,
            out: *mut ::guatiao::Text,
            err: *mut ::guatiao::library::ProviderError,
        ) -> ::guatiao::Status,
    >,
    pub count: ::core::option::Option<
        unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            bytes: ::guatiao::value::types::Bytes,
            flag: bool,
            out: *mut i64,
        ) -> ::guatiao::Status,
    >,
}
#[allow(non_snake_case, clippy::missing_safety_doc)]
impl GreeterVtable {
    /// The table for one implementation of the kind.
    #[doc(hidden)]
    pub const fn of<__T: Greeter>() -> Self {
        Self {
            header: ::guatiao::library::KindHeader::new(
                ::core::mem::size_of::<Self>(),
                <dyn Greeter as ::guatiao::library::Kind>::FLOOR_HASH,
            ),
            greet: ::core::option::Option::Some(Self::__guatiao_shim_greet::<__T>),
            count: ::core::option::Option::Some(Self::__guatiao_shim_count::<__T>),
        }
    }
    /// One past the last required slot. Frozen.
    #[doc(hidden)]
    pub const fn floor() -> usize {
        Self::greet_end()
    }
    #[doc(hidden)]
    pub const fn greet_end() -> usize {
        ::core::mem::offset_of!(Self, greet) + ::core::mem::size_of::<usize>()
    }
    #[doc(hidden)]
    pub const fn count_end() -> usize {
        ::core::mem::offset_of!(Self, count) + ::core::mem::size_of::<usize>()
    }
    /// # Safety
    ///
    /// Called through the table `of::<__T>()` built, with the `ctx`
    /// that table's provider declared: a `&__T`.
    #[doc(hidden)]
    pub unsafe extern "C" fn __guatiao_shim_greet<__T: Greeter>(
        ctx: *mut ::core::ffi::c_void,
        name: ::guatiao::Str,
        out: *mut ::guatiao::Text,
        err: *mut ::guatiao::library::ProviderError,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
            let name = match unsafe { ::guatiao::library::kind::str_arg(name) } {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__s) => {
                    return unsafe {
                        ::guatiao::library::kind::write_err(err, __s.into())
                    };
                }
            };
            match __this.greet(name) {
                ::core::result::Result::Ok(__answer) => {
                    unsafe {
                        ::guatiao::library::kind::write_out(
                            out,
                            ::guatiao::Text::new(&__answer),
                        )
                    }
                }
                ::core::result::Result::Err(__e) => {
                    unsafe { ::guatiao::library::kind::write_err(err, __e) }
                }
            }
        })
    }
    /// # Safety
    ///
    /// Called through the table `of::<__T>()` built, with the `ctx`
    /// that table's provider declared: a `&__T`.
    #[doc(hidden)]
    pub unsafe extern "C" fn __guatiao_shim_count<__T: Greeter>(
        ctx: *mut ::core::ffi::c_void,
        bytes: ::guatiao::value::types::Bytes,
        flag: bool,
        out: *mut i64,
    ) -> ::guatiao::Status {
        ::guatiao::library::kind::catch(|| {
            let ::core::option::Option::Some(__this) = (unsafe {
                ::guatiao::library::kind::ctx_ref::<__T>(ctx)
            }) else {
                return ::guatiao::Status::GUATIAO_ERR_NULL;
            };
            let bytes = match unsafe { ::guatiao::library::kind::bytes_arg(bytes) } {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__s) => {
                    return __s;
                }
            };
            unsafe {
                ::guatiao::library::kind::write_out(out, __this.count(bytes, flag))
            }
        })
    }
}
impl ::guatiao::library::Kind for dyn Greeter {
    const NAME: &'static str = "greeter";
    type Vtable = GreeterVtable;
    const FLOOR: usize = GreeterVtable::floor();
    const FLOOR_HASH: u32 = ::guatiao::library::kind::fnv1a(
        "greet(&str)->Result<String,ProviderError>",
    );
    const REQUIRED: &'static [(&'static str, usize)] = &[
        ("greet", GreeterVtable::greet_end()),
    ];
    fn as_dyn(remote: &::guatiao::library::Remote<Self>) -> &Self {
        remote
    }
    fn boxed(remote: ::guatiao::library::Remote<Self>) -> ::std::boxed::Box<Self> {
        ::std::boxed::Box::new(remote)
    }
    fn shared(remote: ::guatiao::library::Remote<Self>) -> ::std::sync::Arc<Self> {
        ::std::sync::Arc::new(remote)
    }
}
#[allow(clippy::needless_question_mark)]
impl Greeter for ::guatiao::library::Remote<dyn Greeter> {
    fn greet(&self, name: &str) -> Result<String, ProviderError> {
        let __f: unsafe extern "C" fn(
            ctx: *mut ::core::ffi::c_void,
            name: ::guatiao::Str,
            out: *mut ::guatiao::Text,
            err: *mut ::guatiao::library::ProviderError,
        ) -> ::guatiao::Status = unsafe {
            self.slot(
                ::core::mem::offset_of!(GreeterVtable, greet),
                GreeterVtable::greet_end(),
            )
        }
            .expect("validated: a required slot is present and non-null");
        let __arg_name = ::guatiao::Str::borrowed(name);
        let mut __out = ::guatiao::Text::new("");
        let mut __err = ::guatiao::library::ProviderError::none();
        let __status = unsafe { __f(self.ctx(), __arg_name, &mut __out, &mut __err) };
        if __status != ::guatiao::Status::GUATIAO_OK {
            return ::core::result::Result::Err(
                ::guatiao::library::kind::take_err(__err, __status),
            );
        }
        ::core::result::Result::Ok(
            ::std::string::ToString::to_string(__out.as_str().unwrap_or("")),
        )
    }
    fn count(&self, bytes: &[u8], flag: bool) -> i64 {
        match unsafe {
            self.slot::<
                    unsafe extern "C" fn(
                        ctx: *mut ::core::ffi::c_void,
                        bytes: ::guatiao::value::types::Bytes,
                        flag: bool,
                        out: *mut i64,
                    ) -> ::guatiao::Status,
                >(
                ::core::mem::offset_of!(GreeterVtable, count),
                GreeterVtable::count_end(),
            )
        } {
            ::core::option::Option::Some(__f) => {
                let __arg_bytes = ::guatiao::value::types::Bytes {
                    ptr: bytes.as_ptr(),
                    len: bytes.len(),
                };
                let __arg_flag = flag;
                let mut __out: i64 = ::core::default::Default::default();
                let __status = unsafe {
                    __f(self.ctx(), __arg_bytes, __arg_flag, &mut __out)
                };
                if __status != ::guatiao::Status::GUATIAO_OK {
                    ::core::panic!(
                        "guatiao: the provider failed `{}`, which cannot fail: {}",
                        "count", ::core::format_args!("{:?}", __status)
                    );
                }
                __out
            }
            ::core::option::Option::None => bytes.len() as i64,
        }
    }
}
impl ::core::convert::From<::guatiao::library::Remote<dyn Greeter>>
for ::std::boxed::Box<dyn Greeter> {
    fn from(remote: ::guatiao::library::Remote<dyn Greeter>) -> Self {
        ::std::boxed::Box::new(remote)
    }
}
