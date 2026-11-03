// Which page a menu id shows, and which components the Shell mounts once.
//
// Wave 0 owns this file so that packages never edit it: a package replaces the body of its
// placeholder in place. `PAGES` maps a menu id (navigation.js) to a page component; `EXTRAS`
// lists the overlays `Shell.jsx` renders after the content (palette, alerts, help, tour...).

import { lazy } from 'solid-js';

const SandboxListPage = lazy(() => import('../sandboxes/SandboxListPage.jsx'));
const TrafficPage = lazy(() => import('../traffic/TrafficPage.jsx'));
const PoliciesPage = lazy(() => import('../policies/PoliciesPage.jsx'));
const ClusterPage = lazy(() => import('../cluster/ClusterPage.jsx'));
const ProvidersPage = lazy(() => import('../providers/ProvidersPage.jsx'));
const ProxiesPage = lazy(() => import('../proxies/ProxiesPage.jsx'));
const ModelsPage = lazy(() => import('../models/ModelsPage.jsx'));
const NodesPage = lazy(() => import('../nodes/NodesPage.jsx'));
const RecipesPage = lazy(() => import('../recipes/RecipesPage.jsx'));
const PersistentPage = lazy(() => import('../persistent/PersistentPage.jsx'));
const UsersPage = lazy(() => import('../users/UsersPage.jsx'));
const LogsPage = lazy(() => import('../logs/LogsPage.jsx'));
const McpPage = lazy(() => import('../mcp/McpPage.jsx'));
const MediaPage = lazy(() => import('../media/MediaPage.jsx'));
const PrivacyPage = lazy(() => import('../privacy/PrivacyPage.jsx'));
const MasterControlPage = lazy(() => import('../master/MasterControlPage.jsx'));
const UsagePage = lazy(() => import('../usage/UsagePage.jsx'));
const LivePage = lazy(() => import('../live/LivePage.jsx'));
const BuilderPage = lazy(() => import('../builder/BuilderPage.jsx'));
const JobsPage = lazy(() => import('../jobs/JobsPage.jsx'));
const SkillsPage = lazy(() => import('../skills/SkillsPage.jsx'));
const CudaXPage = lazy(() => import('../cudax/CudaXPage.jsx'));
const SystemPage = lazy(() => import('../system/SystemPage.jsx'));
const HostingPage = lazy(() => import('../hosting/HostingPage.jsx'));
const MovesPage = lazy(() => import('../moves/MovesPage.jsx'));
const ImagesPage = lazy(() => import('../images/ImagesPage.jsx'));
const TenantsPage = lazy(() => import('../tenants/TenantsPage.jsx'));
const ClustersPage = lazy(() => import('../clusters/ClustersPage.jsx'));
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
