import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zh from "./zh.json";

// v1 仅中文资源；框架预留多语言扩展（spec: 明确不做多语言）。
export const resources = { zh: { translation: zh } } as const;

void i18n.use(initReactI18next).init({
  resources,
  lng: "zh",
  fallbackLng: "zh",
  interpolation: {
    escapeValue: false,
  },
});

export default i18n;
