use std::{
    mem,
    ptr::NonNull,
    sync::{RwLock, RwLockReadGuard},
};

use once_cell::sync::OnceCell;
use rquickjs::{
    atom::PredefinedAtom,
    function::Constructor,
    qjs::{self, JSValue, JSValueUnion, JS_DupContext, JS_DupValue},
    Ctx, Exception, Function, Object, Result, Value,
};

pub struct ObjectCache {
    ctx: NonNull<qjs::JSContext>,
    cache: [qjs::JSValue; 256],
}

static OBJECT_CACHE: OnceCell<RwLock<Option<ObjectCache>>> = OnceCell::new();

trait EnumIndex {
    fn index(self) -> usize;
}

pub enum ConstructorCacheKey {
    Map,
    Set,
    Date,
    Error,
    RegExp,
    Buffer,
}

pub enum PrototypeCacheKey {
    Object,
    Date,
    RegExp,
    Set,
    Map,
    Error,
}

pub enum FunctionCacheKey {
    ArrayFrom,
    ArrayBufferIsView,
    GetOwnPropertyDescriptor,
}

impl EnumIndex for PrototypeCacheKey {
    fn index(self) -> usize {
        self as usize
    }
}

impl EnumIndex for ConstructorCacheKey {
    fn index(self) -> usize {
        (self as usize) + (1 << 6)
    }
}

impl EnumIndex for FunctionCacheKey {
    fn index(self) -> usize {
        (self as usize) + (1 << 7)
    }
}

impl ObjectCache {
    #[inline(always)]
    pub fn get<'a>() -> RwLockReadGuard<'a, Option<ObjectCache>> {
        let cache = OBJECT_CACHE.get().unwrap();
        cache.read().unwrap()
    }

    pub fn get_function<'js>(&self, key: FunctionCacheKey) -> Result<Function<'js>> {
        Function::from_value(self.get_value(key))
    }

    #[allow(dead_code)]
    pub fn get_prototype<'js>(&self, key: PrototypeCacheKey) -> Result<Object<'js>> {
        Object::from_value(self.get_value(key))
    }

    pub fn get_constructor<'js>(&self, key: ConstructorCacheKey) -> Result<Constructor<'js>> {
        Constructor::from_value(self.get_value(key))
    }

    #[inline(always)]
    fn get_value<'js>(&self, key: impl EnumIndex) -> Value<'js> {
        let ctx = unsafe { Ctx::from_raw(self.ctx) };
        let cached_value = self.cache[key.index()];
        let js_value = unsafe { JS_DupValue(cached_value) };
        unsafe { Value::from_raw(ctx, js_value) }
    }
}

fn append_cache(cache: &mut [JSValue; 256], map: impl EnumIndex, object: Object<'_>) {
    let ctx = object.ctx();
    let value = object.as_raw();
    unsafe { qjs::JS_FreeContext(ctx.as_raw().as_ptr()) };
    mem::forget(object);

    cache[map.index()] = value
}

pub fn clear() {
    let cache = OBJECT_CACHE.get().unwrap();

    if let Some(cache) = cache.write().unwrap().take() {
        for value in cache.cache {
            unsafe {
                if !value.u.ptr.is_null() {
                    qjs::JS_FreeValue(cache.ctx.as_ptr(), value);
                }
            }
        }
        unsafe {
            qjs::JS_FreeContext(cache.ctx.as_ptr());
        }
    }
}

pub fn init(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();

    let object_ctor: Object = globals.get(PredefinedAtom::Object)?;
    let object_proto: Object = object_ctor.get(PredefinedAtom::Prototype)?;

    let get_own_property_desc_fn: Object =
        object_ctor.get(PredefinedAtom::GetOwnPropertyDescriptor)?;

    let date_ctor: Object = globals.get(PredefinedAtom::Date)?;
    let date_proto: Object = date_ctor.get(PredefinedAtom::Prototype)?;

    let map_ctor: Object = globals.get(PredefinedAtom::Map)?;
    let map_proto: Object = map_ctor.get(PredefinedAtom::Prototype)?;

    let set_ctor: Object = globals.get(PredefinedAtom::Set)?;
    let set_proto: Object = set_ctor.get(PredefinedAtom::Prototype)?;

    let reg_exp_ctor: Object = globals.get(PredefinedAtom::RegExp)?;
    let reg_exp_proto: Object = reg_exp_ctor.get(PredefinedAtom::Prototype)?;

    let error_ctor: Object = globals.get(PredefinedAtom::Error)?;
    let error_proto: Object = error_ctor.get(PredefinedAtom::Prototype)?;

    let array_ctor: Object = globals.get(PredefinedAtom::Array)?;
    let array_from_fn: Object = array_ctor.get(PredefinedAtom::From)?;

    let array_buffer_ctor: Object = globals.get(PredefinedAtom::ArrayBuffer)?;
    let array_buffer_is_view_fn: Object = array_buffer_ctor.get("isView")?;

    let buffer_ctor: Object = globals.get(stringify!(Buffer))?;

    let mut values: [JSValue; 256] = [JSValue {
        u: JSValueUnion { int32: 0 },
        tag: -1,
    }; 256];

    //constructors
    append_cache(&mut values, ConstructorCacheKey::Map, map_ctor);
    append_cache(&mut values, ConstructorCacheKey::Set, set_ctor);
    append_cache(&mut values, ConstructorCacheKey::Error, error_ctor);
    append_cache(&mut values, ConstructorCacheKey::RegExp, reg_exp_ctor);
    append_cache(&mut values, ConstructorCacheKey::Date, date_ctor);
    append_cache(&mut values, ConstructorCacheKey::Buffer, buffer_ctor);

    //functions
    append_cache(&mut values, FunctionCacheKey::ArrayFrom, array_from_fn);
    append_cache(
        &mut values,
        FunctionCacheKey::GetOwnPropertyDescriptor,
        get_own_property_desc_fn,
    );
    append_cache(
        &mut values,
        FunctionCacheKey::ArrayBufferIsView,
        array_buffer_is_view_fn,
    );

    //prototypes
    append_cache(&mut values, PrototypeCacheKey::Map, map_proto);
    append_cache(&mut values, PrototypeCacheKey::Set, set_proto);
    append_cache(&mut values, PrototypeCacheKey::Error, error_proto);
    append_cache(&mut values, PrototypeCacheKey::RegExp, reg_exp_proto);
    append_cache(&mut values, PrototypeCacheKey::Date, date_proto);
    append_cache(&mut values, PrototypeCacheKey::Object, object_proto);

    let ctx_ptr = ctx.as_raw();
    let ctx_ptr = unsafe { JS_DupContext(ctx_ptr.as_ptr()) };

    let cache = ObjectCache {
        ctx: unsafe { NonNull::new_unchecked(ctx_ptr) },
        cache: values,
    };

    OBJECT_CACHE
        .set(RwLock::new(Some(cache)))
        .map_err(|_| Exception::throw_message(ctx, "ObjectCache already inited!"))?;
    Ok(())
}

unsafe impl Send for ObjectCache {}
unsafe impl Sync for ObjectCache {}
