import type { RefObject } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  ChevronLeft,
  MessageSquare,
  Pin,
  PinOff,
  Search,
  Settings as SettingsIcon,
  Smile,
  Tag,
  Trash2,
  X
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { getTagColor, getTagTextColor } from "../../../shared/lib/utils";

interface AppHeaderProps {
  t: (key: string) => string;
  showSettings: boolean;
  setShowSettings: (val: boolean) => void;
  showTagManager: boolean;
  setShowTagManager: (val: boolean) => void;
  tagManagerEnabled: boolean;
  showEmojiPanel: boolean;
  setShowEmojiPanel: (val: boolean) => void;
  emojiPanelEnabled: boolean;
  chatMode: boolean;
  fileServerEnabled: boolean;
  isWindowPinned: boolean;
  setIsWindowPinned: (val: boolean) => void;
  clearHistory: () => void;
  showSearchBox: boolean;
  search: string;
  setSearch: (val: string) => void;
  isComposing?: boolean;
  setIsComposing: (val: boolean) => void;
  searchInputRef: RefObject<HTMLInputElement | null>;
  showTagFilter: boolean;
  setShowTagFilter: (val: boolean) => void;
  allTags: string[];
  searchIsFocused: boolean;
  setSearchIsFocused: (val: boolean) => void;
  setEditingTagsId: (val: number | null) => void;
  theme: string;
  colorMode: string;
  settingsTitle: string;
  typeFilter: string | null;
  setTypeFilter: (val: string | null) => void;
  onBack: () => void;
  onToggleChat: () => void;
}

const AppHeader = ({
  t,
  showSettings,
  setShowSettings,
  showTagManager,
  setShowTagManager,
  tagManagerEnabled,
  showEmojiPanel,
  setShowEmojiPanel,
  emojiPanelEnabled,
  chatMode,
  fileServerEnabled,
  isWindowPinned,
  setIsWindowPinned,
  clearHistory,
  showSearchBox,
  search,
  setSearch,
  isComposing = false,
  setIsComposing,
  searchInputRef,
  showTagFilter,
  setShowTagFilter,
  allTags,
  searchIsFocused,
  setSearchIsFocused,
  setEditingTagsId,
  theme,
  colorMode,
  settingsTitle,
  typeFilter,
  setTypeFilter,
  onBack,
  onToggleChat
}: AppHeaderProps) => {
  const getTypeName = (type: string) => {
    switch (type) {
      case "all": return t('type_all') || '全部';
      case "code": return t('type_code');
      case "link":
      case "url": return t('type_url');
      case "file": return t('type_file');
      case "image": return t('type_image');
      case "video": return t('type_video');
      case "rich_text": return t('type_rich_text');
      default: return t('type_text') || 'Text';
    }
  };

  return (
    <header className="window-drag-region">
      <div className="header-top">
        <div className="header-leading">
          {(showSettings || showTagManager || showEmojiPanel) && (
            <button className="btn-icon window-no-drag" onClick={onBack} title={t('back') || '返回'}>
              <ChevronLeft size={16} />
            </button>
          )}
          <div className="header-drag-region" data-tauri-drag-region>
            <span className="header-title">
              {showEmojiPanel
                ? (t('emoji_panel') || '表情包')
                : showTagManager && tagManagerEnabled
                  ? (t('tag_manager') || '标签管理')
                  : showSettings
                    ? settingsTitle
                    : t('app_name')}
            </span>
          </div>
        </div>
        <div className="header-actions window-no-drag">
          <button
            className={`btn-icon ${isWindowPinned ? 'active' : ''}`}
            title={isWindowPinned ? (t('unpin') || '取消置顶') : (t('pin') || '置顶窗口')}
            onClick={() => {
              const newVal = !isWindowPinned;
              setIsWindowPinned(newVal);
              invoke("set_window_pinned", { pinned: newVal }).catch(console.error);
            }}
          >
            {isWindowPinned ? <PinOff size={15} /> : <Pin size={15} />}
          </button>

          {!showSettings && !showTagManager && !showEmojiPanel && (
            <>
              <button className="btn-icon" title={t('clear_history') || '清空历史'} onClick={clearHistory}>
                <Trash2 size={15} />
              </button>
              {tagManagerEnabled && (
                <button className="btn-icon" title={t('tag_manager') || '标签管理'} onClick={() => setShowTagManager(true)}>
                  <Tag size={15} />
                </button>
              )}
              {emojiPanelEnabled && (
                <button className="btn-icon" title={t('emoji_panel') || '表情包'} onClick={() => setShowEmojiPanel(true)}>
                  <Smile size={15} />
                </button>
              )}
              <button className="btn-icon" title={t('settings') || '设置'} onClick={() => setShowSettings(true)}>
                <SettingsIcon size={15} />
              </button>
            </>
          )}
          {fileServerEnabled && (
            <button
              className={`btn-icon header-chat-btn ${chatMode && showSettings ? 'active' : ''}`}
              title="Chat"
              onClick={onToggleChat}
            >
              <MessageSquare size={15} />
            </button>
          )}
          <button
            className="btn-icon btn-header-close"
            title={t('hide') || '隐藏窗口'}
            onClick={async () => {
              invoke("hide_window_cmd").catch(console.error);
            }}
          >
            <X size={15} />
          </button>
        </div>
      </div>

      {!showSettings && !showTagManager && !showEmojiPanel && (
        <AnimatePresence initial={false}>
          {(showSearchBox || search.trim().length > 0) && (
            <motion.div
              initial={{ height: 0, opacity: 0, overflow: 'hidden' }}
              animate={{
                height: "auto",
                opacity: 1,
                transitionEnd: { overflow: "visible" }
              }}
              exit={{ height: 0, opacity: 0, overflow: 'hidden' }}
              transition={{ duration: 0.16, ease: "easeOut" }}
              style={{ flexShrink: 0 }}
            >
              <div className="search-container window-no-drag">
                <div className="search-input-wrapper">
                  <Search size={14} className="search-icon" />
                  <input
                    ref={searchInputRef}
                    type="text"
                    className={`search-input ${showTagFilter && allTags.length > 0 ? 'dropdown-open' : ''}`}
                    placeholder={t('search_placeholder')}
                    value={search}
                    onCompositionStart={() => setIsComposing(true)}
                    onCompositionEnd={(e) => {
                      setIsComposing(false);
                      setSearch((e.target as HTMLInputElement).value);
                    }}
                    onChange={(e) => {
                      setSearch(e.target.value);
                    }}
                    onKeyDown={(e) => {
                      if (isComposing || e.nativeEvent.isComposing || e.keyCode === 229) {
                        return;
                      }
                      if (e.key === "Escape" && search) {
                        e.stopPropagation();
                        setSearch("");
                      }
                    }}
                    onMouseDown={() => {
                      invoke("activate_window_focus").catch(console.error);
                    }}
                    onClick={() => { setShowTagFilter(true); setEditingTagsId(null); }}
                    onFocus={() => {
                      invoke("activate_window_focus").catch(console.error);
                      setShowTagFilter(true);
                      setSearchIsFocused(true);
                      setEditingTagsId(null);
                    }}
                    onBlur={() => {
                      setTimeout(() => {
                        setShowTagFilter(false);
                        setSearchIsFocused(false);
                      }, 200);
                    }}
                    style={{ color: colorMode === 'dark' ? '#ffffff' : undefined }}
                  />
                  {search.length > 0 && (
                    <button
                      type="button"
                      className="search-clear-btn"
                      onClick={() => {
                        setSearch("");
                        searchInputRef.current?.focus();
                      }}
                      title={t('clear') || '清除搜索'}
                    >
                      <X size={12} />
                    </button>
                  )}
                  {showTagFilter && searchIsFocused && allTags.length > 0 && (
                    <div className="tags-dropdown">
                      <div className="tags-label">{t('tags') || "Tags"}</div>
                      <div className="tags-list">
                        {allTags.map(tag => {
                          const tagBackground = getTagColor(tag, theme);
                          return (
                            <span
                              className="tag-chip"
                              key={tag}
                              onMouseDown={(e) => {
                                e.preventDefault();
                                setSearch("tag:" + tag);
                                setShowTagFilter(false);
                              }}
                              data-tag={tag}
                              style={{ background: tagBackground, color: getTagTextColor(tagBackground) }}
                            >
                              {tag}
                            </span>
                          );
                        })}
                      </div>
                    </div>
                  )}
                </div>
                <div
                  className="type-filter-bar"
                  onWheel={(e) => {
                    if (e.deltaY !== 0) {
                      e.currentTarget.scrollLeft += e.deltaY;
                    }
                  }}
                >
                  <button
                    key="all"
                    type="button"
                    className={`type-filter-chip ${typeFilter === null ? 'active' : ''}`}
                    onClick={() => setTypeFilter(null)}
                  >
                    {getTypeName("all")}
                  </button>
                  {['text', 'image', 'file', 'url', 'code', 'video', 'rich_text'].map(tKey => (
                    <button
                      key={tKey}
                      type="button"
                      className={`type-filter-chip ${typeFilter === tKey ? 'active' : ''}`}
                      onClick={() => setTypeFilter(typeFilter === tKey ? null : tKey)}
                      title={getTypeName(tKey)}
                    >
                      {getTypeName(tKey)}
                    </button>
                  ))}
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      )}
    </header>
  );
};

export default AppHeader;
