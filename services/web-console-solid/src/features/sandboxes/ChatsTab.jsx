// The model conversations of this sandbox, as a messenger thread.
import ChatBrowser from '../chats/ChatBrowser.jsx';

export default function ChatsTab(props) {
  return <ChatBrowser sandboxId={props.sandbox?.id} />;
}
