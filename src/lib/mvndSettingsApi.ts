// mvnd settings.xml (票 18) 的前端接口封装：内容是配置而非机密，明文回传
// 做编辑回填；持久化与磁盘物化（<appdata>/mvnd-settings.xml）都由后端完成，
// 前端从不直接写文件。

import { invoke } from "@tauri-apps/api/core";

/** mvnd settings.xml 状态（设置页卡片）。content 为 null 表示未设置。 */
export type MvndSettingsXml = {
  content: string | null;
  fileExists: boolean;
};

export function fetchMvndSettingsXml(): Promise<MvndSettingsXml> {
  return invoke("get_mvnd_settings_xml");
}

/** 保存 settings.xml 内容（后端写入加密库并物化到磁盘）。 */
export function saveMvndSettingsXml(content: string): Promise<MvndSettingsXml> {
  return invoke("set_mvnd_settings_xml", { content });
}

/** 清空 settings.xml（恢复默认，构建不再追加 -s）。 */
export function clearMvndSettingsXml(): Promise<MvndSettingsXml> {
  return invoke("clear_mvnd_settings_xml");
}
