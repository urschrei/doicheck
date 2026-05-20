import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const listDocuments = () => invoke("list_documents");
export const getEmail = () => invoke("get_email");
export const setEmail = (email) => invoke("set_email", { email });
export const openDocument = (path) => invoke("open_document", { path });
export const reportByFingerprint = (fingerprint) =>
  invoke("report_by_fingerprint", { fingerprint });
export const checkDocument = (path) => invoke("check_document", { path });
export const exportReport = (path, text) => invoke("export_report", { path, text });
export const onProgress = (handler) => listen("progress", (e) => handler(e.payload));
