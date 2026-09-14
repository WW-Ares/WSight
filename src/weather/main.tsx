import "../styles/global.css";
import { boot } from "../shared/boot";
import { WeatherPanel } from "./WeatherPanel";

void boot((cfg) => <WeatherPanel config={cfg} />, { widget: "weather" });
