/**
 * Heroes by display name. The game files call one hero by several folder names (Grey Talon is `orion`
 * in the hero table and `archer` or `grey_talon` in model folders), so each hero lists all of them; the
 * first is the key of the game's own hero table (`scripts/heroes.vdata`).
 */
export type Hero = { name: string; codes: string[] };

export const HEROES: Hero[] = [
  { name: "Abrams", codes: ["atlas", "bull", "abrams", "atlas_detective"] },
  { name: "Apollo", codes: ["fencer", "apollo"] },
  { name: "Baba", codes: ["baba"] },
  { name: "Bebop", codes: ["bebop"] },
  { name: "Billy", codes: ["punkgoat", "billy"] },
  { name: "Calico", codes: ["nano", "calico"] },
  { name: "Celeste", codes: ["unicorn", "celeste"] },
  { name: "Deadman Danny", codes: ["deadpack", "deadman_danny"] },
  { name: "Doorman", codes: ["doorman", "the_doorman"] },
  { name: "Drifter", codes: ["drifter"] },
  { name: "Dynamo", codes: ["dynamo", "sumo", "prof_dynamo"] },
  { name: "Graves", codes: ["necro", "graves"] },
  { name: "Grey Talon", codes: ["orion", "archer", "grey_talon", "greytalon"] },
  { name: "Haze", codes: ["haze"] },
  { name: "Holliday", codes: ["astro", "holliday"] },
  { name: "Infernus", codes: ["inferno", "infernus"] },
  { name: "Ivy", codes: ["tengu", "ivy"] },
  { name: "Kelvin", codes: ["kelvin"] },
  { name: "Lady Geist", codes: ["spectre", "ghost", "lady_geist"] },
  { name: "Lash", codes: ["lash"] },
  { name: "McGinnis", codes: ["forge", "engineer", "mcginnis"] },
  { name: "Mina", codes: ["vampirebat", "mina"] },
  { name: "Mirage", codes: ["mirage"] },
  { name: "Mo & Krill", codes: ["krill", "digger", "mo_and_krill", "mo_krill"] },
  { name: "Nurse Harrow", codes: ["nurse", "nurse_harrow"] },
  { name: "Paige", codes: ["bookworm", "paige"] },
  { name: "Paradox", codes: ["chrono", "paradox"] },
  { name: "Pocket", codes: ["synth", "pocket"] },
  { name: "Rat King", codes: ["ratking", "rat_king"] },
  { name: "Rem", codes: ["familiar", "rem"] },
  { name: "Seven", codes: ["gigawatt", "seven"] },
  { name: "Shiv", codes: ["shiv"] },
  { name: "Silver", codes: ["werewolf", "silver"] },
  { name: "Sinclair", codes: ["magician", "sinclair"] },
  { name: "Solomon", codes: ["chessmaster", "solomon"] },
  { name: "Venator", codes: ["priest", "venator"] },
  { name: "Victor", codes: ["frank", "victor"] },
  { name: "Vindicta", codes: ["hornet", "vindicta"] },
  { name: "Violet", codes: ["artist", "violet"] },
  { name: "Viscous", codes: ["viscous"] },
  { name: "Vyper", codes: ["viper", "vyper"] },
  { name: "Warden", codes: ["warden"] },
  { name: "Wraith", codes: ["wraith"] },
  { name: "Wrecker", codes: ["butcher", "wrecker"] },
  { name: "Yamato", codes: ["yamato"] },
];

/** `doorman_v2` -> `doorman`: version suffixes and the `hero_` prefix are not part of the name. */
const clean = (code: string) =>
  code
    .toLowerCase()
    .replace(/^hero_/, "")
    .replace(/_?v\d+$/, "");

/** The hero a folder name belongs to, or null for folders that are not a hero (`backgrounds`, `common`). */
export function heroOfCode(code: string): Hero | null {
  const c = clean(code);
  return HEROES.find((h) => h.codes.includes(c) || h.codes.some((x) => x.replace(/_/g, "") === c.replace(/_/g, ""))) ?? null;
}
