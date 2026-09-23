(ns jepsen.openraft.scenario.leader-removal-recovery
  "Recover an uncommitted leader removal through automatic campaigning (#2091)."
  (:require [clojure.string :as str]
            [jepsen [checker :as checker]
             [control :as c]
             [db :as db]
             [generator :as gen]
             [nemesis :as nemesis]]
            [jepsen.openraft [await :as await]
             [client :as http]
             [cluster :as cluster]
             [generator :as guarded]]))

(def recovery-ms 60000)
(def ^:private chain "OR_LEADER_REMOVAL")
(def ^:private sentinel-key "leader-removal-sentinel")
(def ^:private fresh-key "leader-removal-fresh")

(defn- voters [membership]
  (mapv set (get-in membership [:membership :configs])))

(defn window-valid? [nodes [a b]]
  (let [[a-id b-id] (map http/node-host nodes)
        f (:membership_config a)
        j (:committed_membership_config a)
        index (get-in f [:log_id :index])]
    (boolean
     (and (integer? index)
          (= [#{b-id}] (voters f))
          (= #{a-id b-id} (set (map name (keys (get-in f [:membership :nodes])))))
          (= [#{a-id b-id} #{b-id}] (voters j))
          (= j (:membership_config b) (:committed_membership_config b))
          (< (get-in j [:log_id :index] Long/MAX_VALUE) index)
          (< (get-in a [:local_committed :index] Long/MAX_VALUE) index)
          (< (:last_log_index b Long/MAX_VALUE) index)
          (every? #(<= (get-in j [:log_id :index])
                       (get-in % [:last_applied :index] -1)) [a b])))))

(defn final-ready? [node f metrics]
  (and (some? (:log_id f))
       (= "Leader" (:state metrics))
       (= (http/node-host node) (:current_leader metrics))
       (= f (:membership_config metrics) (:committed_membership_config metrics))
       (<= (get-in f [:log_id :index]) (get-in metrics [:last_applied :index] -1))))

(defn- heal! [test]
  (c/on-nodes test
              (fn [_ _]
                (c/su
                 ;; A failed listing is a control-plane error, not evidence of absence.
                 (let [rules (str/split-lines (c/exec :iptables :-w :-S))]
                   (when (some #{(str "-N " chain)} rules)
                     (doseq [_ (filter #{(str "-A OUTPUT -j " chain)} rules)]
                       (c/exec :iptables :-w :-D :OUTPUT :-j chain))
                     (c/exec :iptables :-w :-F chain)
                     (c/exec :iptables :-w :-X chain)))))))

(defn- partition! [test]
  (c/on-nodes test
              (fn [_ _]
                (c/su
                 (c/exec :iptables :-w :-N chain)
                 (c/exec :iptables :-w :-A chain :-p :tcp :--dport (:raft-port test 22001) :-j :DROP)
                 (c/exec :iptables :-w :-I :OUTPUT :-j chain)
                 (c/exec :iptables :-w :-C :OUTPUT :-j chain)
                 (c/exec :iptables :-w :-C chain :-p :tcp :--dport (:raft-port test 22001) :-j :DROP)))))

(defn- await-state! [test condition pred]
  (await/until!
   condition
   #(let [metrics (mapv (partial cluster/node-metrics! test) (:nodes test))]
      (if (pred metrics) metrics (await/retry! condition {:metrics metrics})))
   {:timeout 30000 :retry-interval 100}))

(defn- prepare! [test]
  (let [[a b] (:nodes test)
        endpoint (http/api-endpoint test a)
        joint [(mapv http/node-host [a b]) [(http/node-host b)]]]
    (http/write! endpoint sentinel-key "before-crash")
    ;; append_membership writes exactly one entry; it never automatically appends F.
    (http/append-membership! endpoint joint)
    (await-state! test :joint-applied
                  #(and (apply = (map :committed_membership_config %))
                        (every? (fn [m]
                                  (let [j (:committed_membership_config m)]
                                    (and (= (mapv set joint) (voters j))
                                         (= j (:membership_config m))
                                         (<= (get-in j [:log_id :index])
                                             (get-in m [:last_applied :index] -1))))) %)))
    (partition! test)
    (try
      (http/append-membership! endpoint [[(http/node-host b)]])
      (catch Exception e
        ;; The uncommitted append may time out. Only the metrics below prove coverage.
        (when-not (= :request-timeout (:kind (ex-data e))) (throw e))))
    (await-state! test :final-window #(window-valid? (:nodes test) %))))

(defn- recover! [test f started]
  (let [b (second (:nodes test))
        endpoint (http/api-endpoint test b)
        last-observation (atom nil)
        elapsed #(quot (- (System/nanoTime) started) 1000000)]
    (try
      (let [result
            (await/until!
             :leader-removal-recovery
             #(try
                (let [metrics (cluster/node-metrics! test b)]
                  (reset! last-observation {:metrics metrics})
                  (when-not (final-ready? b f metrics)
                    (await/retry! :leader-removal-recovery @last-observation))
                  ;; Direct calls to B: no redirection or dependence on A's application API.
                  (http/write! endpoint fresh-key "after-heal")
                  (assoc @last-observation
                         :fresh (:value (http/linearizable-read! endpoint fresh-key))
                         :sentinel (:value (http/linearizable-read! endpoint sentinel-key))))
                (catch Exception e
                  (if (or (#{:unreachable :request-timeout :transport-error} (:kind (ex-data e)))
                          (and (= :openraft-error (:kind (ex-data e)))
                               (some #{:ForwardToLeader :QuorumNotEnough} (keys (:error (ex-data e))))))
                    (await/retry! :leader-removal-recovery (assoc @last-observation :error (ex-data e)))
                    (throw e))))
             {:timeout (max 1 (- recovery-ms (elapsed))) :retry-interval 100})]
        (assoc result :status :recovered :elapsed-ms (elapsed)))
      (catch Exception e
        (if (await/condition-timeout? e :leader-removal-recovery)
          (assoc @last-observation :status :timeout :elapsed-ms (elapsed))
          (throw e))))))

(defn verdict [test history]
  (let [events (filter #(and (= :nemesis (:process %)) (= :info (:type %))) history)
        event #(last (filter (fn [op] (= % (:f op))) events))
        prepared (event :prepare-leader-removal)
        restarted (event :restart-removed-leader)
        healed (event :heal-leader-removal)
        recovered (event :check-leader-removal)
        steps [prepared restarted healed recovered]
        window (get-in prepared [:value :metrics])
        result (:value recovered)
        covered? (and (= :prepared (get-in prepared [:value :status]))
                      (window-valid? (:nodes test) window)
                      (= :restarted (get-in restarted [:value :status]))
                      (= :healed (get-in healed [:value :status])))
        recovered? (and (= :recovered (:status result))
                        (<= 0 (:elapsed-ms result Long/MAX_VALUE) recovery-ms)
                        (final-ready? (second (:nodes test)) (:membership_config (first window)) (:metrics result))
                        (= "after-heal" (:fresh result))
                        (= "before-crash" (:sentinel result)))]
    {:valid? (boolean (and covered? recovered? (every? some? steps) (apply < (map :time steps))))
     :scenario-covered? (boolean covered?) :recovered? (boolean recovered?)
     :recovery result}))

(defn package [database]
  (let [context (atom {})]
    {:nemesis
     (reify nemesis/Nemesis
       (setup! [this test] (heal! test) this)
       (invoke! [_ test op]
         (assoc op :value
                (case (:f op)
                  :prepare-leader-removal
                  (let [metrics (prepare! test)]
                    (swap! context assoc :final (:membership_config (first metrics)))
                    {:status :prepared :metrics metrics})
                  :restart-removed-leader
                  (do (c/on-nodes test [(first (:nodes test))]
                                  (fn [test node]
                                    (db/kill! database test node)
                                    (db/start! database test node)))
                      {:status :restarted})
                  :heal-leader-removal
                  (do (heal! test)
                      (swap! context assoc :healed-at (System/nanoTime))
                      {:status :healed})
                  :check-leader-removal
                  (recover! test (:final @context) (:healed-at @context)))))
       (teardown! [_ test] (heal! test)))
     :checker (reify checker/Checker
                (check [_ test history _] (verdict test history)))}))

(defn generator [failure-state]
  (let [op #(gen/once {:type :info :f %})]
    (gen/nemesis
     (gen/phases
      (guarded/stop-on-harness-failure
       failure-state (gen/phases (op :prepare-leader-removal) (op :restart-removed-leader)))
      (op :heal-leader-removal)
      (guarded/stop-on-harness-failure failure-state (op :check-leader-removal))))))
