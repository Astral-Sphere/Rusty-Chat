-- Rusty-Chat Postgres bootstrap DDL
-- Ground truth: pg_dump --schema-only of a database migrated to head
-- d4c1a8e37b62 by open-webui 0.11.3's own alembic chain (postgres 17).
-- psql meta-commands stripped; alembic_version stamped at the end.

CREATE TABLE public.access_grant (
    id text NOT NULL,
    resource_type text NOT NULL,
    resource_id text NOT NULL,
    principal_type text NOT NULL,
    principal_id text NOT NULL,
    permission text NOT NULL,
    created_at bigint NOT NULL
);
CREATE TABLE public.alembic_version (
    version_num character varying(32) NOT NULL
);
CREATE TABLE public.api_key (
    id text NOT NULL,
    user_id text,
    key text NOT NULL,
    data json,
    expires_at bigint,
    last_used_at bigint,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.auth (
    id character varying NOT NULL,
    email character varying,
    password text,
    active boolean
);
CREATE TABLE public.automation (
    id text NOT NULL,
    user_id text NOT NULL,
    name text NOT NULL,
    data json NOT NULL,
    meta json,
    is_active boolean NOT NULL,
    last_run_at bigint,
    next_run_at bigint,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL,
    folder_id text
);
CREATE TABLE public.automation_run (
    id text NOT NULL,
    automation_id text NOT NULL,
    chat_id text,
    status text NOT NULL,
    error text,
    created_at bigint NOT NULL
);
CREATE TABLE public.calendar (
    id text NOT NULL,
    user_id text NOT NULL,
    name text NOT NULL,
    color text,
    is_default boolean NOT NULL,
    data json,
    meta json,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.calendar_event (
    id text NOT NULL,
    calendar_id text NOT NULL,
    user_id text NOT NULL,
    title text NOT NULL,
    description text,
    start_at bigint NOT NULL,
    end_at bigint,
    all_day boolean NOT NULL,
    rrule text,
    color text,
    location text,
    data json,
    meta json,
    is_cancelled boolean NOT NULL,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.calendar_event_attendee (
    id text NOT NULL,
    event_id text NOT NULL,
    user_id text NOT NULL,
    status text NOT NULL,
    meta json,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.channel (
    id text NOT NULL,
    user_id text,
    name text,
    description text,
    data json,
    meta json,
    created_at bigint,
    updated_at bigint,
    type text,
    is_private boolean,
    archived_at bigint,
    archived_by text,
    deleted_at bigint,
    deleted_by text,
    updated_by text
);
CREATE TABLE public.channel_file (
    id text NOT NULL,
    user_id text NOT NULL,
    channel_id text NOT NULL,
    file_id text NOT NULL,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL,
    message_id text
);
CREATE TABLE public.channel_member (
    id text NOT NULL,
    channel_id text NOT NULL,
    user_id text NOT NULL,
    created_at bigint,
    status text,
    is_active boolean DEFAULT true NOT NULL,
    is_channel_muted boolean DEFAULT false NOT NULL,
    is_channel_pinned boolean DEFAULT false NOT NULL,
    data json,
    meta json,
    joined_at bigint NOT NULL,
    left_at bigint,
    last_read_at bigint,
    updated_at bigint,
    role text,
    invited_by text,
    invited_at bigint
);
CREATE TABLE public.channel_webhook (
    id text NOT NULL,
    user_id text NOT NULL,
    channel_id text NOT NULL,
    name text NOT NULL,
    profile_image_url text,
    token text NOT NULL,
    last_used_at bigint,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.chat (
    id character varying NOT NULL,
    user_id character varying,
    title text,
    created_at bigint,
    updated_at bigint,
    share_id text,
    archived boolean,
    chat json,
    pinned boolean,
    meta json DEFAULT '{}'::json NOT NULL,
    folder_id text,
    tasks json,
    summary text,
    last_read_at bigint,
    current_message_id text,
    variables json,
    timer_at bigint
);
CREATE TABLE public.chat_file (
    id text NOT NULL,
    user_id text NOT NULL,
    chat_id text NOT NULL,
    file_id text NOT NULL,
    message_id text,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.chat_message (
    id text NOT NULL,
    chat_id text NOT NULL,
    user_id text,
    role text NOT NULL,
    parent_id text,
    content json,
    output json,
    model_id text,
    files json,
    sources json,
    embeds json,
    done boolean,
    status_history json,
    error json,
    usage json,
    created_at bigint,
    updated_at bigint,
    context_summary text,
    meta json
);
CREATE TABLE public.chatidtag (
    id character varying NOT NULL,
    tag_name character varying,
    chat_id character varying,
    user_id character varying,
    "timestamp" bigint
);
CREATE TABLE public.config (
    key text NOT NULL,
    value json NOT NULL,
    updated_at bigint
);
CREATE TABLE public.config_old (
    id integer NOT NULL,
    data json NOT NULL,
    version integer NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now()
);
CREATE SEQUENCE public.config_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;
ALTER SEQUENCE public.config_id_seq OWNED BY public.config_old.id;
CREATE TABLE public.document (
    collection_name character varying NOT NULL,
    name character varying,
    title text,
    filename text,
    content text,
    user_id character varying,
    "timestamp" bigint
);
CREATE TABLE public.feedback (
    id text NOT NULL,
    user_id text,
    version bigint,
    type text,
    data json,
    meta json,
    snapshot json,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.file (
    id character varying NOT NULL,
    user_id character varying,
    filename text,
    meta json,
    created_at bigint,
    hash text,
    data json,
    updated_at bigint,
    path text
);
CREATE TABLE public.folder (
    id text NOT NULL,
    parent_id text,
    user_id text NOT NULL,
    name text NOT NULL,
    items json,
    meta json,
    is_expanded boolean NOT NULL,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL,
    data json
);
CREATE TABLE public.function (
    id character varying NOT NULL,
    user_id character varying,
    name text,
    type text,
    content text,
    meta text,
    valves text,
    is_active boolean,
    is_global boolean,
    updated_at bigint,
    created_at bigint
);
CREATE TABLE public."group" (
    id text NOT NULL,
    user_id text,
    name text,
    description text,
    data json,
    meta json,
    permissions json,
    created_at bigint,
    updated_at bigint
);
CREATE TABLE public.group_member (
    id text NOT NULL,
    group_id text NOT NULL,
    user_id text NOT NULL,
    created_at bigint,
    updated_at bigint
);
CREATE TABLE public.knowledge (
    id text NOT NULL,
    user_id text NOT NULL,
    name text NOT NULL,
    description text,
    meta json,
    created_at bigint NOT NULL,
    updated_at bigint,
    data json
);
CREATE TABLE public.knowledge_directory (
    id text NOT NULL,
    knowledge_id text NOT NULL,
    parent_id text,
    name text NOT NULL,
    user_id text NOT NULL,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.knowledge_file (
    id text NOT NULL,
    user_id text NOT NULL,
    knowledge_id text NOT NULL,
    file_id text NOT NULL,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL,
    directory_id text
);
CREATE TABLE public.memory (
    id character varying NOT NULL,
    user_id character varying,
    content text,
    updated_at bigint,
    created_at bigint,
    type character varying DEFAULT 'context'::character varying NOT NULL,
    path text,
    meta json
);
CREATE TABLE public.message (
    id text NOT NULL,
    user_id text,
    channel_id text,
    content text,
    data json,
    meta json,
    created_at bigint,
    updated_at bigint,
    parent_id text,
    reply_to_id text,
    is_pinned boolean DEFAULT false NOT NULL,
    pinned_at bigint,
    pinned_by text
);
CREATE TABLE public.message_reaction (
    id text NOT NULL,
    user_id text NOT NULL,
    message_id text NOT NULL,
    name text NOT NULL,
    created_at bigint
);
CREATE TABLE public.model (
    id text NOT NULL,
    user_id text,
    base_model_id text,
    name text,
    params text,
    meta text,
    updated_at bigint,
    created_at bigint,
    is_active boolean DEFAULT true NOT NULL
);
CREATE TABLE public.note (
    id text NOT NULL,
    user_id text,
    title text,
    data json,
    meta json,
    created_at bigint,
    updated_at bigint
);
CREATE TABLE public.oauth_session (
    id text NOT NULL,
    user_id text NOT NULL,
    provider text NOT NULL,
    token text NOT NULL,
    expires_at bigint NOT NULL,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.pinned_note (
    id text NOT NULL,
    user_id text NOT NULL,
    note_id text NOT NULL,
    created_at bigint NOT NULL
);
CREATE TABLE public.prompt (
    id text NOT NULL,
    command character varying,
    user_id character varying NOT NULL,
    name text NOT NULL,
    content text NOT NULL,
    data json,
    meta json,
    is_active boolean DEFAULT true NOT NULL,
    version_id text,
    tags json,
    created_at bigint NOT NULL,
    updated_at bigint NOT NULL
);
CREATE TABLE public.prompt_history (
    id text NOT NULL,
    prompt_id text NOT NULL,
    parent_id text,
    snapshot json NOT NULL,
    user_id text NOT NULL,
    commit_message text,
    created_at bigint NOT NULL
);
CREATE TABLE public.shared_chat (
    id text NOT NULL,
    chat_id text NOT NULL,
    user_id text NOT NULL,
    title text,
    chat json,
    created_at bigint,
    updated_at bigint
);
CREATE TABLE public.skill (
    id character varying NOT NULL,
    user_id character varying NOT NULL,
    name text NOT NULL,
    description text,
    content text NOT NULL,
    meta json,
    is_active boolean NOT NULL,
    updated_at bigint NOT NULL,
    created_at bigint NOT NULL
);
CREATE TABLE public.tag (
    id character varying NOT NULL,
    name character varying,
    user_id character varying NOT NULL,
    meta json
);
CREATE TABLE public.tool (
    id character varying NOT NULL,
    user_id character varying,
    name text,
    content text,
    specs text,
    meta text,
    valves text,
    updated_at bigint,
    created_at bigint
);
CREATE TABLE public."user" (
    id character varying NOT NULL,
    name character varying,
    email character varying,
    role character varying,
    profile_image_url text,
    last_active_at bigint,
    updated_at bigint,
    created_at bigint,
    settings json,
    info json,
    username character varying(50),
    bio text,
    gender text,
    date_of_birth date,
    profile_banner_image_url text,
    timezone character varying,
    presence_state character varying,
    status_emoji character varying,
    status_message text,
    status_expires_at bigint,
    oauth json,
    scim json,
    variables json
);
ALTER TABLE ONLY public.config_old ALTER COLUMN id SET DEFAULT nextval('public.config_id_seq'::regclass);
ALTER TABLE ONLY public.access_grant
    ADD CONSTRAINT access_grant_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.alembic_version
    ADD CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num);
ALTER TABLE ONLY public.api_key
    ADD CONSTRAINT api_key_key_key UNIQUE (key);
ALTER TABLE ONLY public.api_key
    ADD CONSTRAINT api_key_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.auth
    ADD CONSTRAINT auth_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.automation
    ADD CONSTRAINT automation_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.automation_run
    ADD CONSTRAINT automation_run_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.calendar_event_attendee
    ADD CONSTRAINT calendar_event_attendee_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.calendar_event
    ADD CONSTRAINT calendar_event_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.calendar
    ADD CONSTRAINT calendar_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.channel_file
    ADD CONSTRAINT channel_file_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.channel_member
    ADD CONSTRAINT channel_member_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.channel
    ADD CONSTRAINT channel_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.channel_webhook
    ADD CONSTRAINT channel_webhook_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.chat_file
    ADD CONSTRAINT chat_file_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.chat_message
    ADD CONSTRAINT chat_message_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.chat
    ADD CONSTRAINT chat_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.chat
    ADD CONSTRAINT chat_share_id_key UNIQUE (share_id);
ALTER TABLE ONLY public.chatidtag
    ADD CONSTRAINT chatidtag_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.config_old
    ADD CONSTRAINT config_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.config
    ADD CONSTRAINT config_pkey1 PRIMARY KEY (key);
ALTER TABLE ONLY public.document
    ADD CONSTRAINT document_name_key UNIQUE (name);
ALTER TABLE ONLY public.document
    ADD CONSTRAINT document_pkey PRIMARY KEY (collection_name);
ALTER TABLE ONLY public.feedback
    ADD CONSTRAINT feedback_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.file
    ADD CONSTRAINT file_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.folder
    ADD CONSTRAINT folder_pkey PRIMARY KEY (id, user_id);
ALTER TABLE ONLY public.function
    ADD CONSTRAINT function_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.group_member
    ADD CONSTRAINT group_member_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public."group"
    ADD CONSTRAINT group_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.knowledge_directory
    ADD CONSTRAINT knowledge_directory_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.knowledge_file
    ADD CONSTRAINT knowledge_file_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.knowledge
    ADD CONSTRAINT knowledge_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.memory
    ADD CONSTRAINT memory_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.message
    ADD CONSTRAINT message_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.message_reaction
    ADD CONSTRAINT message_reaction_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.model
    ADD CONSTRAINT model_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.note
    ADD CONSTRAINT note_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.oauth_session
    ADD CONSTRAINT oauth_session_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.pinned_note
    ADD CONSTRAINT pinned_note_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.tag
    ADD CONSTRAINT pk_id_user_id PRIMARY KEY (id, user_id);
ALTER TABLE ONLY public.prompt_history
    ADD CONSTRAINT prompt_history_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.prompt
    ADD CONSTRAINT prompt_new_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.shared_chat
    ADD CONSTRAINT shared_chat_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.skill
    ADD CONSTRAINT skill_name_key UNIQUE (name);
ALTER TABLE ONLY public.skill
    ADD CONSTRAINT skill_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.tool
    ADD CONSTRAINT tool_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.access_grant
    ADD CONSTRAINT uq_access_grant_grant UNIQUE (resource_type, resource_id, principal_type, principal_id, permission);
ALTER TABLE ONLY public.channel_file
    ADD CONSTRAINT uq_channel_file_channel_file UNIQUE (channel_id, file_id);
ALTER TABLE ONLY public.chat_file
    ADD CONSTRAINT uq_chat_file_chat_file UNIQUE (chat_id, file_id);
ALTER TABLE ONLY public.calendar_event_attendee
    ADD CONSTRAINT uq_event_attendee UNIQUE (event_id, user_id);
ALTER TABLE ONLY public.group_member
    ADD CONSTRAINT uq_group_member_group_user UNIQUE (group_id, user_id);
ALTER TABLE ONLY public.knowledge_directory
    ADD CONSTRAINT uq_knowledge_directory_knowledge_parent_name UNIQUE (knowledge_id, parent_id, name);
ALTER TABLE ONLY public.knowledge_file
    ADD CONSTRAINT uq_knowledge_file_knowledge_file UNIQUE (knowledge_id, file_id);
ALTER TABLE ONLY public.pinned_note
    ADD CONSTRAINT uq_pinned_note UNIQUE (user_id, note_id);
ALTER TABLE ONLY public."user"
    ADD CONSTRAINT user_pkey PRIMARY KEY (id);
CREATE INDEX chat_message_chat_parent_idx ON public.chat_message USING btree (chat_id, parent_id);
CREATE INDEX chat_message_chat_role_done_idx ON public.chat_message USING btree (chat_id, role, done);
CREATE INDEX chat_message_model_created_idx ON public.chat_message USING btree (model_id, created_at);
CREATE INDEX chat_message_user_created_idx ON public.chat_message USING btree (user_id, created_at);
CREATE INDEX folder_id_idx ON public.chat USING btree (folder_id);
CREATE INDEX folder_id_user_id_idx ON public.chat USING btree (folder_id, user_id);
CREATE INDEX idx_access_grant_principal ON public.access_grant USING btree (principal_type, principal_id);
CREATE INDEX idx_access_grant_resource ON public.access_grant USING btree (resource_type, resource_id);
CREATE INDEX idx_oauth_session_expires_at ON public.oauth_session USING btree (expires_at);
CREATE INDEX idx_oauth_session_user_id ON public.oauth_session USING btree (user_id);
CREATE INDEX idx_oauth_session_user_provider ON public.oauth_session USING btree (user_id, provider);
CREATE INDEX idx_skill_updated_at ON public.skill USING btree (updated_at);
CREATE INDEX idx_skill_user_id ON public.skill USING btree (user_id);
CREATE INDEX is_global_idx ON public.function USING btree (is_global);
CREATE INDEX ix_automation_next_run ON public.automation USING btree (next_run_at);
CREATE INDEX ix_automation_run_automation_id ON public.automation_run USING btree (automation_id);
CREATE INDEX ix_automation_user_folder ON public.automation USING btree (user_id, folder_id);
CREATE INDEX ix_calendar_event_attendee_user ON public.calendar_event_attendee USING btree (user_id, status);
CREATE INDEX ix_calendar_event_calendar ON public.calendar_event USING btree (calendar_id, start_at);
CREATE INDEX ix_calendar_event_user_date ON public.calendar_event USING btree (user_id, start_at);
CREATE INDEX ix_calendar_user ON public.calendar USING btree (user_id);
CREATE INDEX ix_channel_file_channel_id ON public.channel_file USING btree (channel_id);
CREATE INDEX ix_channel_file_file_id ON public.channel_file USING btree (file_id);
CREATE INDEX ix_channel_file_user_id ON public.channel_file USING btree (user_id);
CREATE INDEX ix_chat_file_chat_id ON public.chat_file USING btree (chat_id);
CREATE INDEX ix_chat_file_file_id ON public.chat_file USING btree (file_id);
CREATE INDEX ix_chat_file_message_id ON public.chat_file USING btree (message_id);
CREATE INDEX ix_chat_file_user_id ON public.chat_file USING btree (user_id);
CREATE INDEX ix_chat_message_chat_id ON public.chat_message USING btree (chat_id);
CREATE INDEX ix_chat_message_created_at ON public.chat_message USING btree (created_at);
CREATE INDEX ix_chat_message_model_id ON public.chat_message USING btree (model_id);
CREATE INDEX ix_chat_message_user_id ON public.chat_message USING btree (user_id);
CREATE INDEX ix_group_member_user_id_group_id ON public.group_member USING btree (user_id, group_id);
CREATE INDEX ix_knowledge_directory_knowledge_id ON public.knowledge_directory USING btree (knowledge_id);
CREATE INDEX ix_knowledge_directory_parent_id ON public.knowledge_directory USING btree (parent_id);
CREATE INDEX ix_knowledge_file_directory_id ON public.knowledge_file USING btree (directory_id);
CREATE INDEX ix_knowledge_file_file_id ON public.knowledge_file USING btree (file_id);
CREATE INDEX ix_knowledge_file_knowledge_id ON public.knowledge_file USING btree (knowledge_id);
CREATE INDEX ix_knowledge_file_user_id ON public.knowledge_file USING btree (user_id);
CREATE INDEX ix_memory_id_user_id ON public.memory USING btree (id, user_id);
CREATE INDEX ix_memory_type ON public.memory USING btree (type);
CREATE INDEX ix_memory_user_id ON public.memory USING btree (user_id);
CREATE INDEX ix_prompt_history_prompt_id ON public.prompt_history USING btree (prompt_id);
CREATE UNIQUE INDEX ix_prompt_new_command ON public.prompt USING btree (command);
CREATE INDEX timer_at_idx ON public.chat USING btree (timer_at) WHERE (timer_at IS NOT NULL);
CREATE INDEX updated_at_user_id_idx ON public.chat USING btree (updated_at, user_id);
CREATE UNIQUE INDEX uq_user_email_lower ON public."user" USING btree (lower((email)::text)) WHERE (email IS NOT NULL);
CREATE INDEX user_id_archived_idx ON public.chat USING btree (user_id, archived);
CREATE INDEX user_id_folder_unread_idx ON public.chat USING btree (user_id, folder_id, archived, updated_at, last_read_at, id);
CREATE INDEX user_id_idx ON public.tag USING btree (user_id);
CREATE INDEX user_id_pinned_idx ON public.chat USING btree (user_id, pinned);
CREATE INDEX user_id_timer_at_idx ON public.chat USING btree (user_id, timer_at) WHERE (timer_at IS NOT NULL);
CREATE INDEX user_id_updated_at_id_idx ON public.chat USING btree (user_id, updated_at DESC, id);
ALTER TABLE ONLY public.api_key
    ADD CONSTRAINT api_key_user_id_fkey FOREIGN KEY (user_id) REFERENCES public."user"(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.channel_file
    ADD CONSTRAINT channel_file_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.channel(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.channel_file
    ADD CONSTRAINT channel_file_file_id_fkey FOREIGN KEY (file_id) REFERENCES public.file(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.channel_webhook
    ADD CONSTRAINT channel_webhook_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES public.channel(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.chat_file
    ADD CONSTRAINT chat_file_chat_id_fkey FOREIGN KEY (chat_id) REFERENCES public.chat(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.chat_file
    ADD CONSTRAINT chat_file_file_id_fkey FOREIGN KEY (file_id) REFERENCES public.file(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.chat_message
    ADD CONSTRAINT chat_message_chat_id_fkey FOREIGN KEY (chat_id) REFERENCES public.chat(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.channel_file
    ADD CONSTRAINT fk_channel_file_message_id FOREIGN KEY (message_id) REFERENCES public.message(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.knowledge_file
    ADD CONSTRAINT fk_knowledge_file_directory_id FOREIGN KEY (directory_id) REFERENCES public.knowledge_directory(id) ON DELETE SET NULL;
ALTER TABLE ONLY public.group_member
    ADD CONSTRAINT group_member_group_id_fkey FOREIGN KEY (group_id) REFERENCES public."group"(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.group_member
    ADD CONSTRAINT group_member_user_id_fkey FOREIGN KEY (user_id) REFERENCES public."user"(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.knowledge_directory
    ADD CONSTRAINT knowledge_directory_knowledge_id_fkey FOREIGN KEY (knowledge_id) REFERENCES public.knowledge(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.knowledge_directory
    ADD CONSTRAINT knowledge_directory_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES public.knowledge_directory(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.knowledge_file
    ADD CONSTRAINT knowledge_file_file_id_fkey FOREIGN KEY (file_id) REFERENCES public.file(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.knowledge_file
    ADD CONSTRAINT knowledge_file_knowledge_id_fkey FOREIGN KEY (knowledge_id) REFERENCES public.knowledge(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.oauth_session
    ADD CONSTRAINT oauth_session_user_id_fkey FOREIGN KEY (user_id) REFERENCES public."user"(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.pinned_note
    ADD CONSTRAINT pinned_note_note_id_fkey FOREIGN KEY (note_id) REFERENCES public.note(id) ON DELETE CASCADE;
ALTER TABLE ONLY public.shared_chat
    ADD CONSTRAINT shared_chat_chat_id_fkey FOREIGN KEY (chat_id) REFERENCES public.chat(id) ON DELETE CASCADE;

INSERT INTO alembic_version (version_num) VALUES ('d4c1a8e37b62');
