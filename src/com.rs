//! Minimal COM bindings over windows-sys for the few Shell interfaces Super
//! Menu consumes. Slot order and GUIDs mirror the Windows SDK headers
//! (ShObjIdl_core.h, ObjIdl.h, shlguid.h); the tests pin the slot layout.
// Slot and method names keep the SDK spelling so each one can be checked
// against its header line.
#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::c_void;
use std::fmt;
use std::marker::PhantomData;
use std::ptr::{null, null_mut, NonNull};
use windows_sys::core::{w, BOOL, GUID, HRESULT, PCWSTR, PWSTR};
use windows_sys::Win32::Foundation::{LocalFree, E_INVALIDARG, E_POINTER, SIZE};
use windows_sys::Win32::Graphics::Gdi::HBITMAP;
use windows_sys::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows_sys::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX, STGM};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FormatMessageW, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_HMODULE,
    FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    SHCreateItemFromParsingName, SHGetKnownFolderItem, KNOWN_FOLDER_FLAG, SIGDN, SIIGBF,
};

/// Failure code of a COM call. Its text is the one the windows crate used to
/// produce, so user-facing diagnostics are unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Error(pub(crate) HRESULT);

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hresult_message(self.0))
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

fn check(status: HRESULT) -> Result<()> {
    // Success codes such as S_FALSE are not failures.
    if status < 0 {
        Err(Error(status))
    } else {
        Ok(())
    }
}

/// System text for an HRESULT followed by its code, or the code alone when
/// Windows has no text for it. NT status codes (FACILITY_NT_BIT) read ntdll's
/// message table, as windows-core did.
pub(crate) fn hresult_message(status: HRESULT) -> String {
    const FACILITY_NT_BIT: u32 = 0x1000_0000;
    let mut code = status as u32;
    let mut flags =
        FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS;
    let mut module = null_mut();
    if code & FACILITY_NT_BIT != 0 {
        code ^= FACILITY_NT_BIT;
        flags |= FORMAT_MESSAGE_FROM_HMODULE;
        module = unsafe { GetModuleHandleW(w!("ntdll.dll")) };
    }
    let mut buffer: PWSTR = null_mut();
    let mut message = String::new();
    unsafe {
        let length = FormatMessageW(
            flags,
            module.cast(),
            code,
            0,
            (&mut buffer as *mut PWSTR).cast(),
            0,
            null(),
        );
        if !buffer.is_null() {
            if length > 0 {
                let mut text = std::slice::from_raw_parts(buffer, length as usize);
                while let Some((32 | 9..=13, rest)) = text.split_last() {
                    text = rest;
                }
                message = String::from_utf16_lossy(text);
            }
            LocalFree(buffer.cast());
        }
    }
    let code = status as u32;
    if message.is_empty() {
        format!("{code:#010X}")
    } else {
        format!("{message} ({code:#010X})")
    }
}

/// Reads a CoTaskMem-allocated string and frees it. Invalid UTF-16 yields
/// None, as the former PWSTR conversion did.
pub(crate) unsafe fn take_co_task_string(text: PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let mut length = 0;
    while *text.add(length) != 0 {
        length += 1;
    }
    let result = String::from_utf16(std::slice::from_raw_parts(text, length)).ok();
    CoTaskMemFree(text.cast());
    result
}

/// A COM interface this crate consumes: its IID and vtable layout. The
/// markers are never instantiated; they only select the vtable behind a pointer.
pub(crate) trait Interface {
    const IID: GUID;
    type Vtable;
}

/// Owning interface pointer. The pointee starts with its vtable pointer, and
/// the null niche lets `Option<ComPtr<T>>` receive raw out-parameters as is.
#[repr(transparent)]
pub(crate) struct ComPtr<T: Interface>(NonNull<c_void>, PhantomData<T>);

impl<T: Interface> ComPtr<T> {
    /// Takes ownership of the reference an out-parameter returned on success.
    unsafe fn from_abi(status: HRESULT, raw: *mut c_void) -> Result<Self> {
        check(status)?;
        NonNull::new(raw)
            .map(|pointer| Self(pointer, PhantomData))
            .ok_or(Error(E_POINTER))
    }

    fn as_raw(&self) -> *mut c_void {
        self.0.as_ptr()
    }

    unsafe fn vtable(&self) -> &T::Vtable {
        &**self.0.as_ptr().cast::<*const T::Vtable>()
    }

    unsafe fn unknown(&self) -> &IUnknown_Vtbl {
        &**self.0.as_ptr().cast::<*const IUnknown_Vtbl>()
    }

    /// QueryInterface for another interface of the same object.
    pub(crate) fn cast<U: Interface>(&self) -> Result<ComPtr<U>> {
        let mut raw = null_mut();
        unsafe {
            let status = (self.unknown().QueryInterface)(self.as_raw(), &U::IID, &mut raw);
            ComPtr::from_abi(status, raw)
        }
    }
}

impl<T: Interface> Drop for ComPtr<T> {
    fn drop(&mut self) {
        unsafe {
            (self.unknown().Release)(self.as_raw());
        }
    }
}

/// CoCreateInstance with no aggregation.
pub(crate) unsafe fn create_instance<T: Interface>(
    class: &GUID,
    context: CLSCTX,
) -> Result<ComPtr<T>> {
    let mut raw = null_mut();
    ComPtr::from_abi(
        CoCreateInstance(class, null_mut(), context, &T::IID, &mut raw),
        raw,
    )
}

/// SHCreateItemFromParsingName with no bind context.
pub(crate) unsafe fn item_from_parsing_name<T: Interface>(path: PCWSTR) -> Result<ComPtr<T>> {
    let mut raw = null_mut();
    ComPtr::from_abi(
        SHCreateItemFromParsingName(path, null_mut(), &T::IID, &mut raw),
        raw,
    )
}

/// SHGetKnownFolderItem for the current user.
pub(crate) unsafe fn known_folder_item<T: Interface>(
    folder: &GUID,
    flags: KNOWN_FOLDER_FLAG,
) -> Result<ComPtr<T>> {
    let mut raw = null_mut();
    ComPtr::from_abi(
        SHGetKnownFolderItem(folder, flags, null_mut(), &T::IID, &mut raw),
        raw,
    )
}

// Vtables: every slot is declared so the layout matches the SDK, even where
// this crate never calls it. Unused slots are naturally never read.
#[allow(dead_code)]
#[repr(C)]
pub(crate) struct IUnknown_Vtbl {
    pub QueryInterface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
}

// ShObjIdl_core.h: MIDL_INTERFACE("43826d1e-e718-42ee-bc55-a1e261c37bfe") IShellItem
pub(crate) enum IShellItem {}

impl Interface for IShellItem {
    const IID: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
    type Vtable = IShellItem_Vtbl;
}

#[allow(dead_code)]
#[repr(C)]
pub(crate) struct IShellItem_Vtbl {
    pub base: IUnknown_Vtbl,
    pub BindToHandler: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const GUID,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    pub GetParent: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub GetDisplayName: unsafe extern "system" fn(*mut c_void, SIGDN, *mut PWSTR) -> HRESULT,
    pub GetAttributes: unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> HRESULT,
    pub Compare: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut i32) -> HRESULT,
}

impl ComPtr<IShellItem> {
    /// Binds without a bind context.
    pub(crate) unsafe fn BindToHandler<U: Interface>(
        &self,
        bhid: *const GUID,
    ) -> Result<ComPtr<U>> {
        let mut raw = null_mut();
        let status =
            (self.vtable().BindToHandler)(self.as_raw(), null_mut(), bhid, &U::IID, &mut raw);
        ComPtr::from_abi(status, raw)
    }

    /// The caller frees the returned string with CoTaskMemFree.
    pub(crate) unsafe fn GetDisplayName(&self, mode: SIGDN) -> Result<PWSTR> {
        let mut name = null_mut();
        check((self.vtable().GetDisplayName)(
            self.as_raw(),
            mode,
            &mut name,
        ))
        .map(|()| name)
    }
}

// ShObjIdl_core.h: MIDL_INTERFACE("bcc18b79-ba16-442f-80c4-8a59c30c463b") IShellItemImageFactory
pub(crate) enum IShellItemImageFactory {}

impl Interface for IShellItemImageFactory {
    const IID: GUID = GUID::from_u128(0xbcc18b79_ba16_442f_80c4_8a59c30c463b);
    type Vtable = IShellItemImageFactory_Vtbl;
}

#[allow(dead_code)]
#[repr(C)]
pub(crate) struct IShellItemImageFactory_Vtbl {
    pub base: IUnknown_Vtbl,
    pub GetImage: unsafe extern "system" fn(*mut c_void, SIZE, SIIGBF, *mut HBITMAP) -> HRESULT,
}

impl ComPtr<IShellItemImageFactory> {
    /// The caller owns the returned bitmap.
    pub(crate) unsafe fn GetImage(&self, size: SIZE, flags: SIIGBF) -> Result<HBITMAP> {
        let mut bitmap = null_mut();
        check((self.vtable().GetImage)(
            self.as_raw(),
            size,
            flags,
            &mut bitmap,
        ))
        .map(|()| bitmap)
    }
}

// ObjIdl.h: MIDL_INTERFACE("0000010b-0000-0000-C000-000000000046") IPersistFile : IPersist
pub(crate) enum IPersistFile {}

impl Interface for IPersistFile {
    const IID: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
    type Vtable = IPersistFile_Vtbl;
}

#[allow(dead_code)]
#[repr(C)]
pub(crate) struct IPersistFile_Vtbl {
    pub base: IUnknown_Vtbl,
    pub GetClassID: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
    pub IsDirty: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub Load: unsafe extern "system" fn(*mut c_void, PCWSTR, STGM) -> HRESULT,
    pub Save: unsafe extern "system" fn(*mut c_void, PCWSTR, BOOL) -> HRESULT,
    pub SaveCompleted: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    pub GetCurFile: unsafe extern "system" fn(*mut c_void, *mut PWSTR) -> HRESULT,
}

impl ComPtr<IPersistFile> {
    pub(crate) unsafe fn Load(&self, file: PCWSTR, mode: STGM) -> Result<()> {
        check((self.vtable().Load)(self.as_raw(), file, mode))
    }

    #[cfg(test)]
    pub(crate) unsafe fn Save(&self, file: PCWSTR, remember: bool) -> Result<()> {
        check((self.vtable().Save)(
            self.as_raw(),
            file,
            BOOL::from(remember),
        ))
    }
}

// ShObjIdl_core.h: MIDL_INTERFACE("000214F9-0000-0000-C000-000000000046") IShellLinkW
pub(crate) enum IShellLinkW {}

impl Interface for IShellLinkW {
    const IID: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    type Vtable = IShellLinkW_Vtbl;
}

#[allow(dead_code)]
#[repr(C)]
pub(crate) struct IShellLinkW_Vtbl {
    pub base: IUnknown_Vtbl,
    pub GetPath:
        unsafe extern "system" fn(*mut c_void, PWSTR, i32, *mut WIN32_FIND_DATAW, u32) -> HRESULT,
    pub GetIDList: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub SetIDList: unsafe extern "system" fn(*mut c_void, *const c_void) -> HRESULT,
    pub GetDescription: unsafe extern "system" fn(*mut c_void, PWSTR, i32) -> HRESULT,
    pub SetDescription: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    pub GetWorkingDirectory: unsafe extern "system" fn(*mut c_void, PWSTR, i32) -> HRESULT,
    pub SetWorkingDirectory: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    pub GetArguments: unsafe extern "system" fn(*mut c_void, PWSTR, i32) -> HRESULT,
    pub SetArguments: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    pub GetHotkey: unsafe extern "system" fn(*mut c_void, *mut u16) -> HRESULT,
    pub SetHotkey: unsafe extern "system" fn(*mut c_void, u16) -> HRESULT,
    pub GetShowCmd: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    pub SetShowCmd: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    pub GetIconLocation: unsafe extern "system" fn(*mut c_void, PWSTR, i32, *mut i32) -> HRESULT,
    pub SetIconLocation: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
    pub SetRelativePath: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT,
    pub Resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> HRESULT,
    pub SetPath: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
}

impl ComPtr<IShellLinkW> {
    /// Fills `file` up to its length, NUL included.
    pub(crate) unsafe fn GetPath(
        &self,
        file: &mut [u16],
        find_data: *mut WIN32_FIND_DATAW,
        flags: u32,
    ) -> Result<()> {
        let count = i32::try_from(file.len()).map_err(|_| Error(E_INVALIDARG))?;
        check((self.vtable().GetPath)(
            self.as_raw(),
            file.as_mut_ptr(),
            count,
            find_data,
            flags,
        ))
    }

    #[cfg(test)]
    pub(crate) unsafe fn SetPath(&self, file: PCWSTR) -> Result<()> {
        check((self.vtable().SetPath)(self.as_raw(), file))
    }
}

// ShObjIdl_core.h: MIDL_INTERFACE("70629033-e363-4a28-a567-0db78006e6d7") IEnumShellItems
pub(crate) enum IEnumShellItems {}

impl Interface for IEnumShellItems {
    const IID: GUID = GUID::from_u128(0x70629033_e363_4a28_a567_0db78006e6d7);
    type Vtable = IEnumShellItems_Vtbl;
}

#[allow(dead_code)]
#[repr(C)]
pub(crate) struct IEnumShellItems_Vtbl {
    pub base: IUnknown_Vtbl,
    pub Next: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32) -> HRESULT,
    pub Skip: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    pub Reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub Clone: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

impl ComPtr<IEnumShellItems> {
    /// Fills `items` from the front; S_FALSE (end reached) is a success.
    pub(crate) unsafe fn Next(
        &self,
        items: &mut [Option<ComPtr<IShellItem>>],
        fetched: &mut u32,
    ) -> Result<()> {
        let count = u32::try_from(items.len()).map_err(|_| Error(E_INVALIDARG))?;
        check((self.vtable().Next)(
            self.as_raw(),
            count,
            items.as_mut_ptr().cast(),
            fetched,
        ))
    }
}

// ShObjIdl_core.h: MIDL_INTERFACE("947aab5f-0a5c-4c13-b4d6-4bf7836fc9f8") IFileOperation
pub(crate) enum IFileOperation {}

impl Interface for IFileOperation {
    const IID: GUID = GUID::from_u128(0x947aab5f_0a5c_4c13_b4d6_4bf7836fc9f8);
    type Vtable = IFileOperation_Vtbl;
}

#[allow(dead_code)]
#[repr(C)]
pub(crate) struct IFileOperation_Vtbl {
    pub base: IUnknown_Vtbl,
    pub Advise: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32) -> HRESULT,
    pub Unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    pub SetOperationFlags: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    pub SetProgressMessage: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    pub SetProgressDialog: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub SetProperties: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub SetOwnerWindow: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub ApplyPropertiesToItem: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub ApplyPropertiesToItems: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub RenameItem:
        unsafe extern "system" fn(*mut c_void, *mut c_void, PCWSTR, *mut c_void) -> HRESULT,
    pub RenameItems: unsafe extern "system" fn(*mut c_void, *mut c_void, PCWSTR) -> HRESULT,
    pub MoveItem: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        PCWSTR,
        *mut c_void,
    ) -> HRESULT,
    pub MoveItems: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    pub CopyItem: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        PCWSTR,
        *mut c_void,
    ) -> HRESULT,
    pub CopyItems: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    pub DeleteItem: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> HRESULT,
    pub DeleteItems: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub NewItem: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        PCWSTR,
        PCWSTR,
        *mut c_void,
    ) -> HRESULT,
    pub PerformOperations: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub GetAnyOperationsAborted: unsafe extern "system" fn(*mut c_void, *mut BOOL) -> HRESULT,
}

impl ComPtr<IFileOperation> {
    pub(crate) unsafe fn SetOperationFlags(&self, flags: u32) -> Result<()> {
        check((self.vtable().SetOperationFlags)(self.as_raw(), flags))
    }

    /// Queues a delete without a progress sink.
    pub(crate) unsafe fn DeleteItem(&self, item: &ComPtr<IShellItem>) -> Result<()> {
        check((self.vtable().DeleteItem)(
            self.as_raw(),
            item.as_raw(),
            null_mut(),
        ))
    }

    pub(crate) unsafe fn PerformOperations(&self) -> Result<()> {
        check((self.vtable().PerformOperations)(self.as_raw()))
    }

    pub(crate) unsafe fn GetAnyOperationsAborted(&self) -> Result<BOOL> {
        let mut aborted = 0;
        check((self.vtable().GetAnyOperationsAborted)(
            self.as_raw(),
            &mut aborted,
        ))
        .map(|()| aborted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn hresult_message_carries_system_text_and_code() {
        let text = hresult_message(0x8007_0002_u32 as HRESULT);
        assert!(text.ends_with(" (0x80070002)"), "{text}");
        assert!(text.len() > " (0x80070002)".len(), "{text}");
        assert!(!text.starts_with(' '), "{text}");
    }

    #[test]
    fn vtables_keep_the_sdk_slot_layout() {
        let slot = size_of::<usize>();
        assert_eq!(size_of::<IUnknown_Vtbl>(), 3 * slot);
        assert_eq!(size_of::<IShellItem_Vtbl>(), 8 * slot);
        assert_eq!(size_of::<IShellItemImageFactory_Vtbl>(), 4 * slot);
        assert_eq!(size_of::<IPersistFile_Vtbl>(), 9 * slot);
        assert_eq!(size_of::<IShellLinkW_Vtbl>(), 21 * slot);
        assert_eq!(size_of::<IEnumShellItems_Vtbl>(), 7 * slot);
        assert_eq!(size_of::<IFileOperation_Vtbl>(), 23 * slot);
        // Slots this crate calls, at the positions the headers give them.
        assert_eq!(offset_of!(IShellItem_Vtbl, BindToHandler), 3 * slot);
        assert_eq!(offset_of!(IShellItem_Vtbl, GetDisplayName), 5 * slot);
        assert_eq!(offset_of!(IShellItemImageFactory_Vtbl, GetImage), 3 * slot);
        assert_eq!(offset_of!(IPersistFile_Vtbl, Load), 5 * slot);
        assert_eq!(offset_of!(IPersistFile_Vtbl, Save), 6 * slot);
        assert_eq!(offset_of!(IShellLinkW_Vtbl, GetPath), 3 * slot);
        assert_eq!(offset_of!(IShellLinkW_Vtbl, SetPath), 20 * slot);
        assert_eq!(offset_of!(IEnumShellItems_Vtbl, Next), 3 * slot);
        assert_eq!(offset_of!(IFileOperation_Vtbl, SetOperationFlags), 5 * slot);
        assert_eq!(offset_of!(IFileOperation_Vtbl, DeleteItem), 18 * slot);
        assert_eq!(
            offset_of!(IFileOperation_Vtbl, PerformOperations),
            21 * slot
        );
        assert_eq!(
            offset_of!(IFileOperation_Vtbl, GetAnyOperationsAborted),
            22 * slot
        );
        assert_eq!(size_of::<Option<ComPtr<IShellItem>>>(), slot);
    }
}
