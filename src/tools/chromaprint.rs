#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use std::borrow::Cow;
use std::error::Error;
use std::ffi::*;
use std::str::Utf8Error;
use std::{mem, slice};

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

const CHROMAPRINT_TRUE: c_int = 1;

pub(crate) fn is_chromaprint_true(value: c_int) -> bool {
    value == CHROMAPRINT_TRUE
}

pub(crate) enum ChromaprintResult<Valid, Invalid, InvalidError> {
    SuccessAndValid(Valid),
    SuccessAndInvalid(Invalid, InvalidError),
    Error(ChromaprintError),
}

#[derive(thiserror::Error, Debug)]
#[error("libchromaprint native function '{function_name}' returned an error.")]
pub(crate) struct ChromaprintError {
    function_name: &'static str,
}

impl ChromaprintError {
    fn for_fn(name: &'static str) -> Self {
        Self {
            function_name: name,
        }
    }
}

impl<Valid, Invalid, InvalidError> ChromaprintResult<Valid, Invalid, InvalidError> {
    fn error(fn_name: &'static str) -> Self {
        ChromaprintResult::Error(ChromaprintError::for_fn(fn_name))
    }
}

impl<Valid, Invalid, InvalidError> ChromaprintResult<Valid, Invalid, InvalidError> {
    fn map_valid<U>(
        self,
        map: impl FnOnce(Valid) -> U,
    ) -> ChromaprintResult<U, Invalid, InvalidError> {
        match self {
            ChromaprintResult::SuccessAndValid(value) => {
                ChromaprintResult::SuccessAndValid(map(value))
            }
            ChromaprintResult::SuccessAndInvalid(inv, inverr) => {
                ChromaprintResult::SuccessAndInvalid(inv, inverr)
            }
            ChromaprintResult::Error(err) => ChromaprintResult::Error(err),
        }
    }
}

impl<Valid, Invalid, InvalidError: Error + 'static>
    ChromaprintResult<Valid, Invalid, InvalidError>
{
    pub(crate) fn into_result(self) -> Result<Valid, Box<dyn Error>> {
        match self {
            ChromaprintResult::SuccessAndValid(value) => Ok(value),
            ChromaprintResult::SuccessAndInvalid(_, err) => Err(Box::new(err)),
            ChromaprintResult::Error(err) => Err(Box::new(err)),
        }
    }
}

type ChromaprintStringResult = ChromaprintResult<String, Box<CStr>, Utf8Error>;
impl ChromaprintStringResult {
    fn success_string(ret: &CStr) -> Self {
        match ret.to_str() {
            Ok(value) => ChromaprintResult::SuccessAndValid(value.to_string()),
            Err(err) => ChromaprintResult::SuccessAndInvalid(Box::from(ret), err),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ChromaprintFingerprint {
    Compressed(Vec<u8>),
    Raw(Vec<u32>, ChromaprintAlgorithm),
    Base64URLSafe(String),
}

impl ChromaprintFingerprint {
    pub(crate) fn from_compressed(value: Vec<u8>) -> Self {
        ChromaprintFingerprint::Compressed(value)
    }

    pub(crate) fn from_raw(value: Vec<u32>, algo: ChromaprintAlgorithm) -> Self {
        ChromaprintFingerprint::Raw(value, algo)
    }

    pub(crate) fn from_base64_urlsafe(value: String) -> Self {
        ChromaprintFingerprint::Base64URLSafe(value)
    }

    pub(crate) fn into_raw_fingerprint(
        self,
    ) -> Result<(Vec<u32>, ChromaprintAlgorithm), ChromaprintError> {
        self.decode().map(|(raw, algo)| (raw.into_owned(), algo))
    }

    pub(crate) fn into_compressed_fingerprint(self) -> Result<Vec<u8>, ChromaprintError> {
        match self {
            ChromaprintFingerprint::Compressed(value) => Ok(value),
            _ => {
                let (decoded, decoded_algo) = self.decode()?;
                let decoded = decoded.into_owned();
                let decoded = decoded.as_slice();
                let decoded_len = decoded.len();
                let decoded: *const c_uint = decoded.as_ptr();

                let mut encoded: *mut c_char = std::ptr::null_mut();
                let mut encoded_len: c_int = 0;

                let success = is_chromaprint_true(unsafe {
                    chromaprint_encode_fingerprint(
                        decoded,
                        decoded_len as c_int,
                        decoded_algo as c_int,
                        &mut encoded,
                        &mut encoded_len,
                        0,
                    )
                });

                if success {
                    let data: &[c_char] =
                        unsafe { slice::from_raw_parts(encoded, encoded_len as usize) };
                    let data_slice = unsafe { mem::transmute::<&[i8], &[u8]>(data) };
                    let res = Vec::from(data_slice);
                    unsafe { chromaprint_dealloc(encoded as *mut c_void) };
                    Ok(res)
                } else {
                    Err(ChromaprintError::for_fn("chromaprint_encode_fingerprint"))
                }
            }
        }
    }

    pub(crate) fn into_base64_urlsafe_fingerprint(
        self,
    ) -> ChromaprintResult<String, Vec<c_char>, Utf8Error> {
        match self {
            ChromaprintFingerprint::Base64URLSafe(value) => {
                ChromaprintResult::SuccessAndValid(value)
            }
            _ => {
                let (decoded, decoded_algo) = match self.decode() {
                    Ok((decoded, algo)) => (decoded.into_owned(), algo),
                    Err(err) => return ChromaprintResult::Error(err),
                };
                let decoded = decoded.as_slice();
                let decoded_len = decoded.len();
                let decoded: *const c_uint = decoded.as_ptr();

                let mut encoded: *mut c_char = std::ptr::null_mut();
                let mut encoded_len: c_int = 0;

                let success = is_chromaprint_true(unsafe {
                    chromaprint_encode_fingerprint(
                        decoded,
                        decoded_len as c_int,
                        decoded_algo as c_int,
                        &mut encoded,
                        &mut encoded_len,
                        1,
                    )
                });

                if success {
                    let data: &[c_char] =
                        unsafe { slice::from_raw_parts(encoded, encoded_len as usize) };
                    let data_slice = unsafe { mem::transmute::<&[i8], &[u8]>(data) };
                    let res = match std::str::from_utf8(data_slice) {
                        Ok(valid) => ChromaprintResult::SuccessAndValid(String::from(valid)),
                        Err(err) => ChromaprintResult::SuccessAndInvalid(Vec::from(data), err),
                    };
                    unsafe { chromaprint_dealloc(encoded as *mut c_void) };
                    res
                } else {
                    ChromaprintResult::error("chromaprint_encode_fingerprint")
                }
            }
        }
    }

    fn decode(&self) -> Result<(Cow<Vec<u32>>, ChromaprintAlgorithm), ChromaprintError> {
        fn decompress<'s>(
            value: impl AsRef<[u8]>,
            base64: bool,
        ) -> Result<(Cow<'s, Vec<u32>>, ChromaprintAlgorithm), ChromaprintError> {
            let compressed = value.as_ref();
            let compressed_len = compressed.len();
            let compressed = compressed.as_ptr();

            let mut decoded: *mut c_uint = std::ptr::null_mut();
            let mut decoded_len: c_int = 0;
            let mut algorithm_ver: c_int = -1;

            let base64: c_int = if base64 { 1 } else { 0 };

            let success = is_chromaprint_true(unsafe {
                chromaprint_decode_fingerprint(
                    compressed as *const c_char,
                    compressed_len as c_int,
                    &mut decoded,
                    &mut decoded_len,
                    &mut algorithm_ver,
                    base64,
                )
            });

            if success {
                let res = Cow::Owned(Vec::from(unsafe {
                    slice::from_raw_parts(decoded, decoded_len as usize)
                }));
                unsafe { chromaprint_dealloc(decoded as *mut c_void) };
                Ok((res, algorithm_ver as ChromaprintAlgorithm))
            } else {
                Err(ChromaprintError::for_fn("chromaprint_decode_fingerprint"))
            }
        }

        match self {
            ChromaprintFingerprint::Compressed(value) => decompress(value, false),
            ChromaprintFingerprint::Raw(value, algo) => Ok((Cow::Borrowed(value), algo.clone())),
            ChromaprintFingerprint::Base64URLSafe(value) => decompress(value.as_bytes(), true),
        }
    }
}

pub(crate) struct Chromaprint {
    ctx: *mut ChromaprintContext,
}

impl Drop for Chromaprint {
    fn drop(&mut self) {
        unsafe { chromaprint_free(self.ctx) }
    }
}

impl Chromaprint {
    pub(crate) fn new_default() -> Self {
        Self::new_with(ChromaprintAlgorithm_CHROMAPRINT_ALGORITHM_DEFAULT)
    }

    pub(crate) fn new_with(algorithm: ChromaprintAlgorithm) -> Self {
        Self {
            ctx: unsafe { chromaprint_new(algorithm as c_int) },
        }
    }

    pub fn version() -> ChromaprintStringResult {
        ChromaprintStringResult::success_string(unsafe {
            CStr::from_ptr(chromaprint_get_version())
        })
    }

    pub fn algorithm(&self) -> ChromaprintAlgorithm {
        let signed = unsafe { chromaprint_get_algorithm(self.ctx) };
        signed as ChromaprintAlgorithm
    }

    pub fn delay_in_samples(&self) -> i32 {
        unsafe { chromaprint_get_delay(self.ctx) }
    }

    pub fn delay(&self) -> std::time::Duration {
        let ms: c_int = unsafe { chromaprint_get_delay_ms(self.ctx) };
        std::time::Duration::from_millis(ms as u64)
    }

    pub fn start(
        &mut self,
        sample_rate: c_int,
        num_channels: c_int,
    ) -> Result<(), ChromaprintError> {
        let success =
            is_chromaprint_true(unsafe { chromaprint_start(self.ctx, sample_rate, num_channels) });
        if success {
            Ok(())
        } else {
            Err(ChromaprintError::for_fn("chromaprint_start"))
        }
    }

    pub fn feed(&mut self, data: &[i16]) -> Result<(), ChromaprintError> {
        let data_len = data.len() as c_int;
        let data = data.as_ptr();
        let success = is_chromaprint_true(unsafe { chromaprint_feed(self.ctx, data, data_len) });
        if success {
            Ok(())
        } else {
            Err(ChromaprintError::for_fn("chromaprint_start"))
        }
    }

    pub fn finish(&mut self) -> Result<(), ChromaprintError> {
        let success = is_chromaprint_true(unsafe { chromaprint_finish(self.ctx) });
        if success {
            Ok(())
        } else {
            Err(ChromaprintError::for_fn("chromaprint_start"))
        }
    }

    pub fn fingerprint(&self) -> ChromaprintResult<ChromaprintFingerprint, Box<CStr>, Utf8Error> {
        let mut fingerprint: *mut c_char = std::ptr::null_mut();
        let success =
            is_chromaprint_true(unsafe { chromaprint_get_fingerprint(self.ctx, &mut fingerprint) });

        if success {
            let cstr = unsafe { CStr::from_ptr(fingerprint) };

            let res = match cstr.to_str() {
                Ok(valid) => ChromaprintResult::SuccessAndValid(
                    ChromaprintFingerprint::from_base64_urlsafe(String::from(valid)),
                ),
                Err(err) => ChromaprintResult::SuccessAndInvalid(Box::from(cstr), err),
            };
            unsafe { chromaprint_dealloc(fingerprint as *mut c_void) }
            res
        } else {
            ChromaprintResult::error("chromaprint_get_fingerprint")
        }
    }

    pub fn fingerprint_hash(&self) -> Result<u32, ChromaprintError> {
        let mut res: u32 = 0;
        let success =
            is_chromaprint_true(unsafe { chromaprint_get_fingerprint_hash(self.ctx, &mut res) });
        if success {
            Ok(res)
        } else {
            Err(ChromaprintError::for_fn("chromaprint_get_fingerprint_hash"))
        }
    }

    pub fn raw_fingerprint(&mut self) -> Result<ChromaprintFingerprint, ChromaprintError> {
        let mut raw_fingerprint: *mut c_uint = std::ptr::null_mut();
        let mut raw_size: c_int = 0;
        let success = is_chromaprint_true(unsafe {
            chromaprint_get_raw_fingerprint(self.ctx, &mut raw_fingerprint, &mut raw_size)
        });

        if success {
            let fingerprint = unsafe { slice::from_raw_parts(raw_fingerprint, raw_size as usize) };

            let res = Ok(ChromaprintFingerprint::from_raw(
                Vec::from(fingerprint),
                self.algorithm(),
            ));
            unsafe { chromaprint_dealloc(raw_fingerprint as *mut c_void) };
            res
        } else {
            Err(ChromaprintError::for_fn("chromaprint_get_raw_fingerprint"))
        }
    }

    pub fn hash_fingerprint(fingerprint: ChromaprintFingerprint) -> Result<u32, ChromaprintError> {
        let mut hash: u32 = 0;
        let (raw, _algo) = fingerprint.into_raw_fingerprint()?;
        let raw_ptr = raw.as_ptr();
        let raw_len = raw.len();

        let success = is_chromaprint_true(unsafe {
            chromaprint_hash_fingerprint(raw_ptr, raw_len as c_int, &mut hash)
        });

        if success {
            Ok(hash)
        } else {
            Err(ChromaprintError::for_fn("chromaprint_hash_fingerprint"))
        }
    }

    pub fn fingerprint_full(
        &mut self,
        sample_rate: c_int,
        num_channels: c_int,
        audio_data: impl IntoIterator<Item = impl AsRef<[i16]>>,
    ) -> Result<ChromaprintFingerprint, ChromaprintError> {
        self.start(sample_rate, num_channels)?;

        let mut audio_data = audio_data.into_iter();
        while let Some(audio_packet) = audio_data.next() {
            self.feed(audio_packet.as_ref())?;
        }

        self.finish()?;
        self.raw_fingerprint()
    }
}
