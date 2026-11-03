// Which page a menu id shows, and which components the Shell mounts once.
//
// Wave 0 owns this file so that packages never edit it: a package replaces the body of its
// placeholder in place. `PAGES` maps a menu id (navigation.js) to a page component; `EXTRAS`
// lists the overlays `Shell.jsx` renders after the content (palette, alerts, help, tour...).

import SandboxListPage from '../sandboxes/SandboxListPage.jsx';
import TrafficPage from '../traffic/TrafficPage.jsx';
import PoliciesPage from '../policies/PoliciesPage.jsx';
import ClusterPage from '../cluster/ClusterPage.jsx';
import ProvidersPage from '../providers/ProvidersPage.jsx';
import ProxiesPage from '../proxies/ProxiesPage.jsx';
import ModelsPage from '../models/ModelsPage.jsx';
import NodesPage from '../nodes/NodesPage.jsx';
import RecipesPage from '../recipes/RecipesPage.jsx';
import PersistentPage from '../persistent/PersistentPage.jsx';
import UsersPage from '../users/UsersPage.jsx';
import LogsPage from '../logs/LogsPage.jsx';
import McpPage from '../mcp/McpPage.jsx';
import MediaPage from '../media/MediaPage.jsx';
import PrivacyPage from '../privacy/PrivacyPage.jsx';
import MasterControlPage from '../master/MasterControlPage.jsx';
import UsagePage from '../usage/UsagePage.jsx';
import LivePage from '../live/LivePage.jsx';
import BuilderPage from '../builder/BuilderPage.jsx';
import JobsPage from '../jobs/JobsPage.jsx';
import SkillsPage from '../skills/SkillsPage.jsx';
import CudaXPage from '../cudax/CudaXPage.jsx';
import SystemPage from '../system/SystemPage.jsx';
import HostingPage from '../hosting/HostingPage.jsx';
import MovesPage from '../moves/MovesPage.jsx';
import ImagesPage from '../images/ImagesPage.jsx';
import TenantsPage from '../tenants/TenantsPage.jsx';
import ClustersPage from '../clusters/ClustersPage.jsx';
import PaletteHost from './PaletteHost.jsx';
import BulkHost from './BulkHost.jsx';
import AlertStack from '../dock/AlertStack.jsx';
import HelpDrawer from '../help/HelpDrawer.jsx';
import ShortcutsSheet from '../help/ShortcutsSheet.jsx';
import Tour from '../help/Tour.jsx';

// Menu id to page component. Every page receives `{ role, onOpenAdmin, onOpenSandbox, onOpenNode }`
// and ignores what it does not need. `home`, `my` and `docs` have their own route kinds.
export const PAGES = {
  sandboxes: SandboxListPage,
  traffic: TrafficPage,
  policies: PoliciesPage,
  cluster: ClusterPage,
  system: SystemPage,
  nodes: NodesPage,
  providers: ProvidersPage,
  proxies: ProxiesPage,
  models: ModelsPage,
  clusters: ClustersPage,
  skills: SkillsPage,
  cudax: CudaXPage,
  builder: BuilderPage,
  recipes: RecipesPage,
  persistent: PersistentPage,
  operations: JobsPage,
  users: UsersPage,
  hosting: HostingPage,
  moves: MovesPage,
  images: ImagesPage,
  tenants: TenantsPage,
  mcp: McpPage,
  media: MediaPage,
  privacy: PrivacyPage,
  master: MasterControlPage,
  live: LivePage,
  usage: UsagePage,
  logs: LogsPage,
};

// Components the Shell mounts once, after the content. Each renders nothing until it has something to show.
export const EXTRAS = [PaletteHost, BulkHost, AlertStack, HelpDrawer, ShortcutsSheet, Tour];
