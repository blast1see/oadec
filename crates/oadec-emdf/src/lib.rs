//! EMDF / Evolution containers, Object Audio Metadata and JOC parameter syntax.
//!
//! Part of the `oadec` object-audio decoder engine.

pub mod container;
pub mod oamd;

pub use container::{
    Container, ContainerError, Flavor, PAYLOAD_ID_JOC, PAYLOAD_ID_OAMD, Payload, PayloadConfig,
};
pub use oamd::{
    BasicInfo, Bed, BedChannel, Element, ElementMd, ExtendedObjectElement, Gain, Oamd, OamdError,
    ObjectElement, ObjectInfoBlock, ProgramAssignment, RenderInfo, Status, TrimElement,
    UpdateTiming,
};
