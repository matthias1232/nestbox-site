// The model conversations of this sandbox, live, in the workbox.
import ChatBrowser from '../chats/ChatBrowser.jsx';

export default function LiveChatsPanel(props) {
  return <ChatBrowser sandboxId={props.sandbox?.id} compact />;
}
