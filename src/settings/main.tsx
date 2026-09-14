import "../styles/global.css";
import { boot } from "../shared/boot";
import { SettingsPanel } from "./SettingsPanel";

void boot((cfg) => <SettingsPanel initial={cfg} />);
