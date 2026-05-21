import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const listDocuments = () => invoke("list_documents");
export const getEmail = () => invoke("get_email");
export const setEmail = (email) => invoke("set_email", { email });
export const getReportsDir = () => invoke("get_reports_dir");
export const setReportsDir = (dir) => invoke("set_reports_dir", { dir });
export const getConcurrency = () => invoke("get_concurrency");
export const setConcurrency = (value) => invoke("set_concurrency", { value });
export const openDocument = (path) => invoke("open_document", { path });
export const latestCheck = (fingerprint) => invoke("latest_check", { fingerprint });
export const checkDocument = (path) => invoke("check_document", { path });
export const exportReport = (path, fingerprint, format) =>
  invoke("export_report", { path, fingerprint, format });
export const recheckFailures = (fingerprint) => invoke("recheck_failures", { fingerprint });
export const deleteDocument = (fingerprint) => invoke("delete_document", { fingerprint });
export const dismissDiscrepancy = (fingerprint, doi, field) =>
  invoke("dismiss_discrepancy", { fingerprint, doi, field });
export const undismissDiscrepancy = (fingerprint, doi, field) =>
  invoke("undismiss_discrepancy", { fingerprint, doi, field });
export const onProgress = (handler) => listen("progress", (e) => handler(e.payload));
export const onOpenAbout = (handler) => listen("open-about", () => handler());
