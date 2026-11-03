-- 0073_sandbox_running - does a sandbox run? A question for jobs that act for no tenant (ADR 0049).
CREATE FUNCTION nb_sandbox_running(p_sandbox uuid) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS
$$ SELECT EXISTS (SELECT 1 FROM sandboxes WHERE id = p_sandbox AND status = 'running') $$;
