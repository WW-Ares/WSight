import "../styles/global.css";
import { boot } from "../shared/boot";
import { MonitorPanel } from "./MonitorPanel";

void boot((cfg) => <MonitorPanel config={cfg} />, { widget: "monitor" });
