import { Show } from 'solid-js';
// Inline SVG icons: one export per icon, 16x16 viewBox, stroke currentColor, drawn for Nestbox (no third-party set).
// `Icon({ name })` picks one by name; unknown names draw nothing.
const PATHS = {
  Home: 'M2 7.5 8 2.5l6 5M3.5 6.5V13.5h9V6.5',
  DataCenter: 'M3 14V2.5h10V14M1.5 14h13M5.5 5h1M9.5 5h1M5.5 8h1M9.5 8h1M7 14v-3h2v3',
  Site: 'M2 8 8 2.5 14 8M3.5 7V13.5h9V7M6.5 13.5v-3.5h3v3.5',
  Node: 'M2.5 2.5h11v3h-11zM2.5 6.5h11v3h-11zM2.5 10.5h11v3h-11zM4.5 4h.01M4.5 8h.01M4.5 12h.01',
  Sandbox: 'M2.5 2.5h11v11h-11z',
  Model: 'M8 1.5 14 5v6l-6 3.5L2 11V5z',
  Cluster: 'M8 2.5h.01M13.5 8h.01M8 13.5h.01M2.5 8h.01M8 3.5v2M12.5 8h-2M8 12.5v-2M3.5 8h2M6.5 8a1.5 1.5 0 1 0 3 0 1.5 1.5 0 1 0-3 0',
  User: 'M5.5 5a2.5 2.5 0 1 0 5 0 2.5 2.5 0 1 0-5 0M2.5 14c0-3 2.5-4.5 5.5-4.5s5.5 1.5 5.5 4.5',
  Policy: 'M8 1.5 13.5 3.5v4c0 3.2-2.3 5.7-5.5 7-3.2-1.3-5.5-3.8-5.5-7v-4zM5.5 8l2 2 3-3.5',
  Traffic: 'M1.5 5h10M9 2.5 11.5 5 9 7.5M14.5 11h-10M7 8.5 4.5 11 7 13.5',
  Logs: 'M3 2.5h10v11H3zM5.5 5.5h5M5.5 8h5M5.5 10.5h3',
  Search: 'M2.5 7a4.5 4.5 0 1 0 9 0 4.5 4.5 0 1 0-9 0M10.5 10.5l3.5 3.5',
  Bell: 'M3.5 11.5h9l-1.2-1.8V7a3.3 3.3 0 0 0-6.6 0v2.7zM6.5 13.5h3',
  Help: 'M2 8a6 6 0 1 0 12 0A6 6 0 1 0 2 8M6.2 6.3A1.9 1.9 0 0 1 8 5c1 0 1.9.7 1.9 1.7 0 1.2-1.9 1.5-1.9 2.8M8 11.3h.01',
  Terminal: 'M1.5 2.5h13v11h-13zM4 6l2.2 2L4 10M7.5 10.5h3.5',
  Desktop: 'M1.5 2.5h13v8.5h-13zM5.5 13.5h5M8 11v2.5',
  Play: 'M4.5 2.5 12.5 8l-8 5.5z',
  Stop: 'M3.5 3.5h9v9h-9z',
  Restart: 'M13 8a5 5 0 1 1-1.6-3.7M13 2.5v3h-3',
  Recreate: 'M2.5 5.5h8l-2-2M13.5 10.5h-8l2 2M2.5 5.5v0M13.5 10.5v0',
  Move: 'M8 1.5v13M1.5 8h13M8 1.5 6 3.5M8 1.5l2 2M8 14.5l-2-2M8 14.5l2-2M1.5 8l2-2M1.5 8l2 2M14.5 8l-2-2M14.5 8l-2 2',
  Trash: 'M2.5 4h11M6 4V2.5h4V4M4 4l.7 9.5h6.6L12 4M6.5 6.5v5M9.5 6.5v5',
  Plus: 'M8 2.5v11M2.5 8h11',
  ChevronRight: 'M6 3l5 5-5 5',
  ChevronDown: 'M3 6l5 5 5-5',
  Check: 'M2.5 8.5 6.5 12.5 13.5 4',
  Close: 'M3 3l10 10M13 3 3 13',
  Warning: 'M8 2 14.5 13.5h-13zM8 6.5v3.5M8 12h.01',
  Lock: 'M3.5 7.5h9v6.5h-9zM5.5 7.5V5a2.5 2.5 0 0 1 5 0v2.5',
  Unlock: 'M3.5 7.5h9v6.5h-9zM5.5 7.5V5a2.5 2.5 0 0 1 4.8-1',
  Eye: 'M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8zM6.3 8a1.7 1.7 0 1 0 3.4 0 1.7 1.7 0 1 0-3.4 0',
  EyeOff: 'M1.5 8S4 3.5 8 3.5c1 0 1.9.3 2.7.8M14.5 8S12 12.5 8 12.5c-1 0-1.9-.3-2.7-.8M2.5 2.5l11 11',
  Pin: 'M9.5 1.5l5 5-2 .5-2.5 2.5.5 3-1 1-3-3-4 4M5.5 6.5l-1 1 4 4',
  PopOut: 'M9 2.5h4.5V7M13.5 2.5 7.5 8.5M11.5 9v4.5h-9v-9H7',
  Filter: 'M2 3h12l-4.5 5.5V13l-3-1.5V8.5z',
  Settings: 'M2 4.5h12M2 8h12M2 11.5h12M5 3v3M10.5 6.5v3M7 10v3',
  Gpu: 'M1.5 4.5h13v7h-13zM4.5 11.5v2M11.5 11.5v2M5.5 8a2 2 0 1 0 4 0 2 2 0 1 0-4 0',
  Memory: 'M1.5 5h13v5h-13zM4 10v2.5M6.5 10v2.5M9 10v2.5M11.5 10v2.5M4 7.5h2M8 7.5h4',
  Disk: 'M2.5 3.5h11v9h-11zM5 10.5h.01M8 6.5a2 2 0 1 0 0 .01',
  Network: 'M6.5 2.5h3v3h-3zM1.5 10.5h3v3h-3zM11.5 10.5h3v3h-3zM8 5.5v2.5M3 10.5V8h10v2.5',
  Clock: 'M2 8a6 6 0 1 0 12 0A6 6 0 1 0 2 8M8 4.5V8l2.5 1.5',
  Copy: 'M5.5 5.5h8v8h-8zM10.5 5.5v-3h-8v8h3',
};

// The shared <svg> wrapper for one path.
function Svg(props) {
  return (
    <svg viewBox="0 0 16 16" width={props.size || 16} height={props.size || 16} fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" class={props.class}>
      <path d={PATHS[props.name]} />
    </svg>
  );
}

// The Home icon.
export function IconHome(props) {
  return <Svg name="Home" {...props} />;
}

// The DataCenter icon.
export function IconDataCenter(props) {
  return <Svg name="DataCenter" {...props} />;
}

// The Site icon.
export function IconSite(props) {
  return <Svg name="Site" {...props} />;
}

// The Node icon.
export function IconNode(props) {
  return <Svg name="Node" {...props} />;
}

// The Sandbox icon.
export function IconSandbox(props) {
  return <Svg name="Sandbox" {...props} />;
}

// The Model icon.
export function IconModel(props) {
  return <Svg name="Model" {...props} />;
}

// The Cluster icon.
export function IconCluster(props) {
  return <Svg name="Cluster" {...props} />;
}

// The User icon.
export function IconUser(props) {
  return <Svg name="User" {...props} />;
}

// The Policy icon.
export function IconPolicy(props) {
  return <Svg name="Policy" {...props} />;
}

// The Traffic icon.
export function IconTraffic(props) {
  return <Svg name="Traffic" {...props} />;
}

// The Logs icon.
export function IconLogs(props) {
  return <Svg name="Logs" {...props} />;
}

// The Search icon.
export function IconSearch(props) {
  return <Svg name="Search" {...props} />;
}

// The Bell icon.
export function IconBell(props) {
  return <Svg name="Bell" {...props} />;
}

// The Help icon.
export function IconHelp(props) {
  return <Svg name="Help" {...props} />;
}

// The Terminal icon.
export function IconTerminal(props) {
  return <Svg name="Terminal" {...props} />;
}

// The Desktop icon.
export function IconDesktop(props) {
  return <Svg name="Desktop" {...props} />;
}

// The Play icon.
export function IconPlay(props) {
  return <Svg name="Play" {...props} />;
}

// The Stop icon.
export function IconStop(props) {
  return <Svg name="Stop" {...props} />;
}

// The Restart icon.
export function IconRestart(props) {
  return <Svg name="Restart" {...props} />;
}

// The Recreate icon.
export function IconRecreate(props) {
  return <Svg name="Recreate" {...props} />;
}

// The Move icon.
export function IconMove(props) {
  return <Svg name="Move" {...props} />;
}

// The Trash icon.
export function IconTrash(props) {
  return <Svg name="Trash" {...props} />;
}

// The Plus icon.
export function IconPlus(props) {
  return <Svg name="Plus" {...props} />;
}

// The ChevronRight icon.
export function IconChevronRight(props) {
  return <Svg name="ChevronRight" {...props} />;
}

// The ChevronDown icon.
export function IconChevronDown(props) {
  return <Svg name="ChevronDown" {...props} />;
}

// The Check icon.
export function IconCheck(props) {
  return <Svg name="Check" {...props} />;
}

// The Close icon.
export function IconClose(props) {
  return <Svg name="Close" {...props} />;
}

// The Warning icon.
export function IconWarning(props) {
  return <Svg name="Warning" {...props} />;
}

// The Lock icon.
export function IconLock(props) {
  return <Svg name="Lock" {...props} />;
}

// The Unlock icon.
export function IconUnlock(props) {
  return <Svg name="Unlock" {...props} />;
}

// The Eye icon.
export function IconEye(props) {
  return <Svg name="Eye" {...props} />;
}

// The EyeOff icon.
export function IconEyeOff(props) {
  return <Svg name="EyeOff" {...props} />;
}

// The Pin icon.
export function IconPin(props) {
  return <Svg name="Pin" {...props} />;
}

// The PopOut icon.
export function IconPopOut(props) {
  return <Svg name="PopOut" {...props} />;
}

// The Filter icon.
export function IconFilter(props) {
  return <Svg name="Filter" {...props} />;
}

// The Settings icon.
export function IconSettings(props) {
  return <Svg name="Settings" {...props} />;
}

// The Gpu icon.
export function IconGpu(props) {
  return <Svg name="Gpu" {...props} />;
}

// The Memory icon.
export function IconMemory(props) {
  return <Svg name="Memory" {...props} />;
}

// The Disk icon.
export function IconDisk(props) {
  return <Svg name="Disk" {...props} />;
}

// The Network icon.
export function IconNetwork(props) {
  return <Svg name="Network" {...props} />;
}

// The Clock icon.
export function IconClock(props) {
  return <Svg name="Clock" {...props} />;
}

// The Copy icon.
export function IconCopy(props) {
  return <Svg name="Copy" {...props} />;
}

// Draw an icon by its name, e.g. <Icon name="Node" />.
export function Icon(props) {
  return <Show when={PATHS[props.name]}><Svg {...props} /></Show>;
}

