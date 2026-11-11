
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS repository (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  git_dir TEXT NOT NULL,
  provenance TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS git_object (
  repo_id INTEGER NOT NULL REFERENCES repository(id),
  oid TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('blob','tree','commit','tag')),
  PRIMARY KEY (repo_id,oid)
);
CREATE TABLE IF NOT EXISTS revision (
  repo_id INTEGER NOT NULL REFERENCES repository(id),
  commit_oid TEXT NOT NULL,
  tree_oid TEXT NOT NULL,
  PRIMARY KEY(repo_id,commit_oid),
  FOREIGN KEY(repo_id,commit_oid) REFERENCES git_object(repo_id,oid),
  FOREIGN KEY(repo_id,tree_oid) REFERENCES git_object(repo_id,oid)
);
CREATE TABLE IF NOT EXISTS tree_entry (
  repo_id INTEGER NOT NULL,
  commit_oid TEXT NOT NULL,
  path TEXT NOT NULL,
  mode TEXT NOT NULL,
  kind TEXT NOT NULL,
  oid TEXT NOT NULL,
  PRIMARY KEY(repo_id,commit_oid,path),
  UNIQUE(repo_id,commit_oid,path,oid),
  FOREIGN KEY(repo_id,commit_oid) REFERENCES revision(repo_id,commit_oid),
  FOREIGN KEY(repo_id,oid) REFERENCES git_object(repo_id,oid)
);
CREATE TABLE IF NOT EXISTS module (
  repo_id INTEGER NOT NULL,
  commit_oid TEXT NOT NULL,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  blob_oid TEXT NOT NULL,
  is_test INTEGER NOT NULL CHECK(is_test IN (0,1)),
  PRIMARY KEY(repo_id,commit_oid,name),
  UNIQUE(repo_id,commit_oid,path),
  FOREIGN KEY(repo_id,commit_oid,path,blob_oid)
    REFERENCES tree_entry(repo_id,commit_oid,path,oid)
);
CREATE TABLE IF NOT EXISTS dependency (
  repo_id INTEGER NOT NULL,
  commit_oid TEXT NOT NULL,
  source_name TEXT NOT NULL,
  target_name TEXT NOT NULL,
  line INTEGER NOT NULL,
  syntax TEXT NOT NULL,
  PRIMARY KEY(repo_id,commit_oid,source_name,target_name,line,syntax),
  FOREIGN KEY(repo_id,commit_oid,source_name)
    REFERENCES module(repo_id,commit_oid,name),
  FOREIGN KEY(repo_id,commit_oid,target_name)
    REFERENCES module(repo_id,commit_oid,name)
);
CREATE INDEX IF NOT EXISTS dependency_reverse ON dependency(repo_id,commit_oid,target_name);

-- Keep SCIP authority distinct from legacy parser observations. This digest
-- binds the imported edge set to a specific decoded SCIP index and tool.
CREATE TABLE IF NOT EXISTS scip_index_metadata (
  repo_id INTEGER NOT NULL,
  commit_oid TEXT NOT NULL,
  index_sha256 TEXT NOT NULL,
  indexer_name TEXT NOT NULL,
  indexer_version TEXT NOT NULL,
  document_count INTEGER NOT NULL CHECK(document_count>=0),
  import_edge_count INTEGER NOT NULL CHECK(import_edge_count>=0),
  PRIMARY KEY (repo_id,commit_oid),
  FOREIGN KEY(repo_id,commit_oid) REFERENCES revision(repo_id,commit_oid)
);

CREATE TRIGGER IF NOT EXISTS check_revision_types BEFORE INSERT ON revision BEGIN
 SELECT CASE WHEN (SELECT kind FROM git_object WHERE repo_id=NEW.repo_id AND oid=NEW.commit_oid) != 'commit'
 THEN RAISE(ABORT,'revision commit must reference a commit object') END;
 SELECT CASE WHEN (SELECT kind FROM git_object WHERE repo_id=NEW.repo_id AND oid=NEW.tree_oid) != 'tree'
 THEN RAISE(ABORT,'revision root must reference a tree object') END;
END;
CREATE TRIGGER IF NOT EXISTS check_entry_types BEFORE INSERT ON tree_entry BEGIN
 SELECT CASE WHEN (SELECT kind FROM git_object WHERE repo_id=NEW.repo_id AND oid=NEW.oid) != NEW.kind
 THEN RAISE(ABORT,'entry kind must agree with Git object kind') END;
 SELECT CASE WHEN (NEW.kind='tree' AND NEW.mode!='040000') OR
                  (NEW.kind='blob' AND NEW.mode NOT IN ('100644','100755','120000')) OR
                  (NEW.kind='commit' AND NEW.mode!='160000')
 THEN RAISE(ABORT,'invalid mode for Git object type') END;
END;
CREATE TRIGGER IF NOT EXISTS check_module_type BEFORE INSERT ON module BEGIN
 SELECT CASE WHEN (SELECT kind FROM tree_entry WHERE repo_id=NEW.repo_id AND commit_oid=NEW.commit_oid AND path=NEW.path)!='blob'
 THEN RAISE(ABORT,'module must refer to blob') END;
END;
CREATE TABLE IF NOT EXISTS symbol (
  repo_id INTEGER NOT NULL,
  commit_oid TEXT NOT NULL,
  module_name TEXT NOT NULL,
  qualname TEXT NOT NULL,
  kind TEXT NOT NULL,
  signature TEXT NOT NULL,
  body_fingerprint TEXT NOT NULL,
  line INTEGER NOT NULL,
  PRIMARY KEY(repo_id,commit_oid,module_name,qualname),
  FOREIGN KEY(repo_id,commit_oid,module_name)
    REFERENCES module(repo_id,commit_oid,name)
);
CREATE TABLE IF NOT EXISTS api_binding (
  repo_id INTEGER NOT NULL,
  commit_oid TEXT NOT NULL,
  module_name TEXT NOT NULL,
  public_name TEXT NOT NULL,
  target TEXT NOT NULL,
  explicitly_exported INTEGER NOT NULL,
  PRIMARY KEY(repo_id,commit_oid,module_name,public_name),
  FOREIGN KEY(repo_id,commit_oid,module_name)
    REFERENCES module(repo_id,commit_oid,name)
);
CREATE TABLE IF NOT EXISTS hazard (
  repo_id INTEGER NOT NULL,
  commit_oid TEXT NOT NULL,
  module_name TEXT NOT NULL,
  line INTEGER NOT NULL,
  reason TEXT NOT NULL,
  PRIMARY KEY(repo_id,commit_oid,module_name,line,reason),
  FOREIGN KEY(repo_id,commit_oid,module_name)
    REFERENCES module(repo_id,commit_oid,name)
);
CREATE TABLE IF NOT EXISTS evidence (
  repo_id INTEGER NOT NULL REFERENCES repository(id),
  cache_key TEXT NOT NULL,
  procedure TEXT NOT NULL,
  procedure_digest TEXT NOT NULL,
  source_digest TEXT NOT NULL,
  environment_digest TEXT NOT NULL,
  inputs_json TEXT NOT NULL,
  command_json TEXT NOT NULL,
  returncode INTEGER NOT NULL,
  output_digest TEXT NOT NULL,
  duration_ms REAL NOT NULL,
  PRIMARY KEY(repo_id,cache_key)
);
CREATE TABLE IF NOT EXISTS verification_observation (
  repo_id INTEGER NOT NULL,
  commit_oid TEXT NOT NULL,
  cache_key TEXT NOT NULL,
  cache_hit INTEGER NOT NULL,
  PRIMARY KEY(repo_id,commit_oid,cache_key),
  FOREIGN KEY(repo_id,commit_oid) REFERENCES revision(repo_id,commit_oid),
  FOREIGN KEY(repo_id,cache_key) REFERENCES evidence(repo_id,cache_key)
);


PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS independent_claim (
 id TEXT PRIMARY KEY,
 repo_id INTEGER NOT NULL REFERENCES repository(id),
 branch_commit TEXT NOT NULL,
 test_suite TEXT NOT NULL,
 observed_count INTEGER NOT NULL CHECK(observed_count>0),
 observation_sha256 TEXT NOT NULL,
 FOREIGN KEY(repo_id,branch_commit) REFERENCES revision(repo_id,commit_oid));
CREATE TABLE IF NOT EXISTS claim_input (
 claim_id TEXT NOT NULL REFERENCES independent_claim(id),
 path TEXT NOT NULL, source_oid TEXT NOT NULL,
 relation TEXT NOT NULL CHECK(relation IN ('defining_source','test_source')),
 PRIMARY KEY(claim_id,path));
CREATE TABLE IF NOT EXISTS integration_result (
 id TEXT PRIMARY KEY,
 repo_id INTEGER NOT NULL REFERENCES repository(id),
 commit_oid TEXT NOT NULL,
 mode TEXT NOT NULL CHECK(mode='git_native_merge'),
 a_claim TEXT NOT NULL REFERENCES independent_claim(id),
 b_claim TEXT NOT NULL REFERENCES independent_claim(id),
 a_disposition TEXT NOT NULL CHECK(a_disposition IN ('BOUNDED_CARRY_ELIGIBLE','REVERIFY')),
 b_disposition TEXT NOT NULL CHECK(b_disposition IN ('BOUNDED_CARRY_ELIGIBLE','REVERIFY')),
 guard_sha256 TEXT NOT NULL,
 assumptions_json TEXT NOT NULL,
 FOREIGN KEY(repo_id,commit_oid) REFERENCES revision(repo_id,commit_oid));
CREATE TABLE IF NOT EXISTS premise (
 integration_id TEXT NOT NULL REFERENCES integration_result(id),
 number INTEGER NOT NULL,
 description TEXT NOT NULL,
 supported INTEGER NOT NULL CHECK(supported=1),
 PRIMARY KEY(integration_id,number));
