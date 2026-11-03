import { addStoryboardMember } from "./transport";
import "../../../apps/web/src/main";
import { setTheme } from "../../../apps/web/src/lib/theme";
import { navigate, parseHash } from "../../../apps/web/src/app/routes";
import { setLanguage } from "../../../apps/web/src/lib/i18n";
import { mountSeats } from "./seats";
const query = new URLSearchParams(location.search);
if (query.has("seats")) mountSeats();
if (query.get("theme") === "dark") setTheme("dark");
if (query.get("lang") === "en") setLanguage("en");
addEventListener("message", (event) => {
  if (parent === window || event.source !== parent) return;
  if (
    event.data?.type === "meerkat-design-theme" &&
    (event.data.theme === "light" || event.data.theme === "dark")
  )
    setTheme(event.data.theme);
  if (
    event.data?.type === "meerkat-design-navigate" &&
    typeof event.data.page === "string"
  )
    navigate(parseHash("#/" + event.data.page));
  if (event.data?.type === "meerkat-design-add-member" && event.data.member)
    addStoryboardMember(event.data.member);
});
new MutationObserver(() => {
  if (parent === window) return;
  parent.postMessage(
    {
      type: "meerkat-design-theme",
      theme: document.documentElement.dataset.theme,
    },
    "*",
  );
}).observe(document.documentElement, {
  attributes: true,
  attributeFilter: ["data-theme"],
});

// Mirror the desktop menu shortcut inside the browser preview.
addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === ",") {
    event.preventDefault();
    navigate({ kind: "settings" });
  }
});
