import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import ru from "./ru.json";
import en from "./en.json";

let lng = "ru";
try {
  lng = localStorage.getItem("lang") ?? "ru";
} catch {
  /* storage can be unavailable */
}

void i18n.use(initReactI18next).init({
  resources: { ru: { translation: ru }, en: { translation: en } },
  lng,
  fallbackLng: "ru",
  interpolation: { escapeValue: false },
});

export default i18n;
