import en from "./locales/en";
import fr from "./locales/fr";
import de from "./locales/de";
import es from "./locales/es";

export type Lang = "en" | "fr" | "de" | "es";
export type Key = keyof typeof en;

/** Noms dans leur propre langue. */
export const LANGS: { code: Lang; name: string }[] = [
  { code: "en", name: "English" },
  { code: "fr", name: "Français" },
  { code: "de", name: "Deutsch" },
  { code: "es", name: "Español" },
];

const dicts: Record<Lang, typeof en> = { en, fr, de, es };
// en-GB plutôt que en-US : heure sur 24 h, jour avant le mois, comme les autres langues.
const LOCALES: Record<Lang, string> = { en: "en-GB", fr: "fr-FR", de: "de-DE", es: "es-ES" };

export const i18n = $state({ lang: "en" as Lang });

export function setLang(lang: Lang) {
  i18n.lang = lang;
  document.documentElement.lang = lang;
}

/** Locale `Intl` de la langue courante. */
export const locale = () => LOCALES[i18n.lang];

/** Texte de la langue courante ; `{nom}` est remplacé par `params.nom`. */
export function t(key: Key, params?: Record<string, string | number>) {
  const s = dicts[i18n.lang][key];
  return params ? s.replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k]) : m)) : s;
}

/** Jour de la semaine, 0 = lundi, en minuscules en milieu de phrase (sauf en allemand, où les noms gardent leur majuscule). */
export const dayNameLower = (d: number) => (i18n.lang === "de" ? dayName(d) : dayName(d).toLowerCase());

/** Jour de la semaine, 0 = lundi. */
export function dayName(d: number) {
  const n = new Date(2024, 0, 1 + d).toLocaleDateString(locale(), { weekday: "long" });
  return n[0].toUpperCase() + n.slice(1);
}
