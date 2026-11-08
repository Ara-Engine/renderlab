"""Native process guard reused from M7-U0b commit 126d513; measurement tooling only."""
import ctypes
from ctypes import wintypes
import json
import os


def native_process_inventory():
    """Win32 snapshot with the same PID/parent/name/path fields as the WMI guard.

    Protected or exited processes may have no readable image path, just as in WMI;
    their names/PIDs remain in the mandatory game/renderer inventory. Other API
    errors reject the snapshot. No process is opened with mutation rights.
    """
    class Entry(ctypes.Structure):
        _fields_=[("size",wintypes.DWORD),("usage",wintypes.DWORD),("pid",wintypes.DWORD),
            ("heap",ctypes.c_size_t),("module",wintypes.DWORD),("threads",wintypes.DWORD),
            ("parent",wintypes.DWORD),("priority",wintypes.LONG),("flags",wintypes.DWORD),
            ("name",wintypes.WCHAR*260)]
    api=ctypes.WinDLL("kernel32",use_last_error=True)
    signatures={"CreateToolhelp32Snapshot":([wintypes.DWORD,wintypes.DWORD],wintypes.HANDLE),
        "Process32FirstW":([wintypes.HANDLE,ctypes.POINTER(Entry)],wintypes.BOOL),
        "Process32NextW":([wintypes.HANDLE,ctypes.POINTER(Entry)],wintypes.BOOL),
        "OpenProcess":([wintypes.DWORD,wintypes.BOOL,wintypes.DWORD],wintypes.HANDLE),
        "QueryFullProcessImageNameW":([wintypes.HANDLE,wintypes.DWORD,wintypes.LPWSTR,ctypes.POINTER(wintypes.DWORD)],wintypes.BOOL),
        "GetExitCodeProcess":([wintypes.HANDLE,ctypes.POINTER(wintypes.DWORD)],wintypes.BOOL),
        "CloseHandle":([wintypes.HANDLE],wintypes.BOOL)}
    for name,(arguments,result) in signatures.items():
        function=getattr(api,name);function.argtypes=arguments;function.restype=result
    def close(handle):
        if not api.CloseHandle(handle):raise ctypes.WinError(ctypes.get_last_error())
    snapshot=api.CreateToolhelp32Snapshot(2,0)  # TH32CS_SNAPPROCESS
    if not snapshot or snapshot==ctypes.c_void_p(-1).value:raise ctypes.WinError(ctypes.get_last_error())
    rows=[]
    try:
        entry=Entry();entry.size=ctypes.sizeof(entry)
        more=api.Process32FirstW(snapshot,ctypes.byref(entry))
        while more:
            row=dict(Name=entry.name,ProcessId=int(entry.pid),ParentProcessId=int(entry.parent),ExecutablePath=None)
            process=api.OpenProcess(0x1000,False,entry.pid)  # PROCESS_QUERY_LIMITED_INFORMATION
            if process:
                try:
                    path=ctypes.create_unicode_buffer(32768);length=wintypes.DWORD(len(path))
                    if api.QueryFullProcessImageNameW(process,0,path,ctypes.byref(length)):
                        row["ExecutablePath"]=path.value
                    else:
                        row["image_query_error"]=ctypes.get_last_error()
                        if row["image_query_error"]==31:
                            status=wintypes.DWORD()
                            if not api.GetExitCodeProcess(process,ctypes.byref(status)):
                                raise ctypes.WinError(ctypes.get_last_error())
                            row["exited_during_image_query"]=status.value!=259  # STILL_ACTIVE
                finally:close(process)
            else:row["image_query_error"]=ctypes.get_last_error()
            if row.get("image_query_error",0) not in (0,5,87,1168) and not row.get("exited_during_image_query"):
                raise RuntimeError("process image query failed: "+json.dumps(row))
            rows.append(row)
            more=api.Process32NextW(snapshot,ctypes.byref(entry))
        if ctypes.get_last_error()!=18:raise ctypes.WinError(ctypes.get_last_error())  # ERROR_NO_MORE_FILES
    finally:close(snapshot)
    if not any(p["ProcessId"]==os.getpid() for p in rows):raise RuntimeError("process snapshot omitted this monitor")
    return rows
