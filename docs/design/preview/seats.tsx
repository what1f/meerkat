// Every desk mood side by side, for review only: open prototype.html?seats.
import { createRoot } from "react-dom/client";
import { Seat, type SeatScreen } from "../../../apps/web/src/scene";
import { avatarUrl } from "../../../apps/web/src/lib/assets";
const crew: [SeatScreen | "away", string, string, "laptop" | "mini"][] = [
  ["ok", "01", "小王", "laptop"],
  ["busy", "11", "阿青", "mini"],
  ["think", "07", "Mia", "mini"],
  ["reply", "14", "Leo", "laptop"],
  ["done", "04", "Ada", "mini"],
  ["boot", "04", "米粒", "laptop"],
  ["alert", "14", "阿哲", "mini"],
  ["asleep", "07", "老周", "laptop"],
  ["dark", "15", "Nova", "mini"],
  ["ok", "15", "小满", "mini"],
  ["away", "01", "Bruce", "laptop"],
];
function Gallery() {
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 1000,
        display: "flex",
        flexWrap: "wrap",
        alignContent: "center",
        justifyContent: "center",
        gap: "56px 40px",
        padding: 48,
        background: "var(--surface)",
      }}
    >
      {crew.map(([screen, figure, name, setup], i) => (
        <Seat
          key={i}
          setup={setup}
          size="hero"
          screen={screen === "away" ? "dark" : screen}
          occupant={screen === "away" ? "empty-chair" : { figure, seed: name }}
          label={name}
          badge={{ name, avatar: avatarUrl(figure) }}
        />
      ))}
    </div>
  );
}
export function mountSeats() {
  const host = document.createElement("div");
  document.body.append(host);
  createRoot(host).render(<Gallery />);
}
