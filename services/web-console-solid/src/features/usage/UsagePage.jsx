// Usage and cost of every model call, for the whole environment.
import UsageView from './UsageView.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

export default function UsagePage() {
  return (
    <div class="page">
      <PageHeader title="Usage &amp; cost" subtitle={<>What every model call was asked and what it cost, for the whole environment.</>} actions={<HelpTip anchor="getting-started" />} />
      <UsageView defaultGroup="model" />
    </div>
  );
}
