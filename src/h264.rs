//! Decodificador H.264 sobre la API de OpenH264 (`openh264-sys2`).
//!
//! El crate `openh264` crea el decodificador con la ocultación de errores
//! desactivada y trata como error cualquier aviso, aunque haya imagen. Con eso, si
//! se pierde una referencia, no vuelve a dar imagen hasta el siguiente fotograma
//! clave, y el iPhone casi nunca los manda: la pantalla se queda congelada. Aquí se
//! activa la ocultación y se entrega la imagen siempre que OpenH264 la produzca.

use std::ptr::{null, null_mut};

use anyhow::{Result, bail};
use openh264_sys2::{
    API, DECODER_OPTION_ERROR_CON_IDC, DECODER_OPTION_TRACE_LEVEL, DynamicAPI, ERROR_CON_SLICE_MV_COPY_CROSS_IDR, ISVCDecoder,
    SBufferInfo, SDecodingParam, SVideoProperty, VIDEO_BITSTREAM_AVC, WELS_LOG_QUIET,
};

/// Estados de OpenH264 que interesan (DECODING_STATE).
pub const DS_NO_PARAM_SETS: i64 = 0x10;
pub const DS_OUT_OF_MEMORY: i64 = 0x4000;

pub struct H264Decoder {
    api: DynamicAPI,
    dec: *mut ISVCDecoder,
}

// El decodificador solo se usa desde un hilo a la vez.
unsafe impl Send for H264Decoder {}

/// Imagen decodificada en YUV 4:2:0. Apunta a la memoria de OpenH264: es válida
/// hasta la siguiente llamada a `decode`.
pub struct Frame<'a> {
    pub width: usize,
    pub height: usize,
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub strides: (usize, usize, usize),
}

impl H264Decoder {
    pub fn new() -> Result<Self> {
        let api = DynamicAPI::from_source();
        let mut dec: *mut ISVCDecoder = null_mut();
        unsafe {
            if api.WelsCreateDecoder(&mut dec) != 0 || dec.is_null() {
                bail!("no se pudo crear el decodificador H.264");
            }
            let vtbl = &**dec;
            let params = SDecodingParam {
                pFileNameRestructed: null_mut(),
                uiCpuLoad: 0,
                uiTargetDqLayer: u8::MAX,
                eEcActiveIdc: ERROR_CON_SLICE_MV_COPY_CROSS_IDR,
                bParseOnly: false,
                sVideoProperty: SVideoProperty { size: std::mem::size_of::<SVideoProperty>() as u32, eVideoBsType: VIDEO_BITSTREAM_AVC },
            };
            let init = vtbl.Initialize.expect("OpenH264: falta Initialize");
            if init(dec, &params) != 0 {
                api.WelsDestroyDecoder(dec);
                bail!("no se pudo iniciar el decodificador H.264");
            }
            let set = vtbl.SetOption.expect("OpenH264: falta SetOption");
            let mut quiet = WELS_LOG_QUIET;
            set(dec, DECODER_OPTION_TRACE_LEVEL, (&raw mut quiet).cast());
            let mut ec = ERROR_CON_SLICE_MV_COPY_CROSS_IDR;
            set(dec, DECODER_OPTION_ERROR_CON_IDC, (&raw mut ec).cast());
        }
        Ok(Self { api, dec })
    }

    /// Decodifica un fragmento Annex B (uno o varios NAL). Devuelve el estado de
    /// OpenH264 (0 = sin incidencias) y la imagen si la hay, aunque el estado avise
    /// de errores ocultados.
    pub fn decode(&mut self, data: &[u8]) -> (i64, Option<Frame<'_>>) {
        let mut dst: [*mut u8; 3] = [null_mut(); 3];
        let mut info: SBufferInfo = unsafe { std::mem::zeroed() };
        let state = unsafe {
            let vtbl = &**self.dec;
            let decode = vtbl.DecodeFrameNoDelay.expect("OpenH264: falta DecodeFrameNoDelay");
            let ptr = if data.is_empty() { null() } else { data.as_ptr() };
            decode(self.dec, ptr, data.len() as i32, dst.as_mut_ptr(), &mut info)
        } as i64;
        if info.iBufferStatus != 1 || dst.iter().any(|p| p.is_null()) {
            return (state, None);
        }
        let buf = unsafe { info.UsrData.sSystemBuffer };
        let (w, h) = (buf.iWidth as usize, buf.iHeight as usize);
        let (ys, cs) = (buf.iStride[0] as usize, buf.iStride[1] as usize);
        if w == 0 || h == 0 {
            return (state, None);
        }
        let frame = unsafe {
            Frame {
                width: w,
                height: h,
                y: std::slice::from_raw_parts(dst[0], ys * h),
                u: std::slice::from_raw_parts(dst[1], cs * h.div_ceil(2)),
                v: std::slice::from_raw_parts(dst[2], cs * h.div_ceil(2)),
                strides: (ys, cs, cs),
            }
        };
        (state, Some(frame))
    }
}

impl Drop for H264Decoder {
    fn drop(&mut self) {
        unsafe {
            if let Some(uninit) = (**self.dec).Uninitialize {
                uninit(self.dec);
            }
            self.api.WelsDestroyDecoder(self.dec);
        }
    }
}
