(ns jepsen.openraft.pre-vote
  "Leader and term stability under a fixed partial network."
  (:require [clojure.string :as str]
            [jepsen [checker :as checker]
             [control :as c]
             [generator :as gen]
             [nemesis :as nemesis]]
            [jepsen.openraft.await :as await]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.liveness :as liveness]
            [jepsen.openraft.db :as db]
            [jepsen.openraft.generator :as generator]
            [jepsen.openraft.harness :as harness]))

(def observation-seconds 20)
(def progress-window-nanos 1500000000)

(defn topology [nodes]
  (when-not (= 5 (count nodes) (count (set nodes)))
    (throw (ex-info "Pre-vote stability requires five distinct nodes" {:nodes nodes})))
  (let [[n1 n2 n3 n4 n5] nodes]
    {:leader n1 :core [n1 n2 n3] :disconnected n4
     :retained [[n1 n2] [n1 n3] [n2 n3] [n2 n4] [n1 n5]]
     ;; Each missing edge needs rules on only one endpoint: INPUT and OUTPUT
     ;; together block both directions, including established TCP connections.
     :blocked {n4 [n1 n3 n5] n5 [n2 n3]}}))

(defn- activity-snapshot! [test]
  ;; Existing INFO events expose leadership changes.
  ;; Counters catch a step-down/re-election even between metric samples.
  (c/on-nodes
   test
   (fn [_ _]
     (let [output (c/exec :awk
                          (str "/ becomes leader/ {up++} "
                               "/ steps down from leader/ {down++} "
                               "END {printf \"%d %d\\n\", up, down}")
                          db/log-file)
           counts (mapv parse-long (str/split (str/trim output) #"\s+"))]
       (when-not (and (= 2 (count counts)) (every? some? counts))
         (throw (ex-info "Invalid pre-vote activity counters" {:output output})))
       (zipmap [:leader-entries :leader-exits] counts)))))

(defn- install! [test plan]
  (c/on-nodes
   test (keys (:blocked plan))
   (fn [_ node]
     (c/su
      (c/exec :iptables :-N :OR_PREVOTE)
      (doseq [chain [:INPUT :OUTPUT]]
        (c/exec :iptables :-I chain :-j :OR_PREVOTE)
        (c/exec :iptables :-C chain :-j :OR_PREVOTE))
      (doseq [peer (get-in plan [:blocked node])
              :let [ip (.getHostAddress (java.net.InetAddress/getByName (http/node-host peer)))]
              direction [:-s :-d]]
        (let [rule [:OR_PREVOTE direction ip :-p :tcp :-m :multiport
                    :--ports (:raft-port test http/default-raft-port) :-j :DROP]]
          (apply c/exec :iptables :-A rule)
          (apply c/exec :iptables :-C rule)))))))

(defn- heal! [test plan]
  (when plan
    (c/on-nodes
     test (keys (:blocked plan))
     (fn [_ _]
       (c/su
        ;; A failed listing must propagate as a control-plane failure; it does
        ;; not establish that the chain is absent or that healing succeeded.
        (let [rules (str/split-lines (c/exec :iptables :-S))]
          (when (some #{"-N OR_PREVOTE"} rules)
            (doseq [chain [:INPUT :OUTPUT]
                    _ (filter #{(str "-A " (name chain) " -j OR_PREVOTE")} rules)]
              (c/exec :iptables :-D chain :-j :OR_PREVOTE))
            (c/exec :iptables :-F :OR_PREVOTE)
            (c/exec :iptables :-X :OR_PREVOTE))))))))

(defn- caught-up? [metrics leader]
  (let [applied (get-in metrics [leader :last_applied])]
    (and applied
         (every? #(and (= applied (:last_applied %))
                       (= (:index applied) (:last_log_index %)))
                 (vals metrics)))))

(defrecord PreVoteStability [roles]
  nemesis/Nemesis
  (setup! [this _] this)
  (invoke! [_ test op]
    (case (:f op)
      :start-pre-vote
      (let [bootstrap (some-> (:bootstrap-state test) deref)
            plan (topology (:nodes test))]
        (when-not bootstrap
          (throw (ex-info "Missing bootstrap readiness result" {})))
        ;; Reuse bootstrap's readiness result; no additional baseline/idle phase.
        (reset! roles plan)
        (let [baseline (assoc bootstrap :activity (activity-snapshot! test))]
          (install! test plan)
          (assoc op :value (assoc plan :status :installed
                                  :baseline baseline :metrics (liveness/metric-snapshot! test (:nodes test))))))

      :sample-pre-vote
      (assoc op :value (cond-> {:status :observed :metrics (liveness/metric-snapshot! test (:nodes test))}
                         (:final-sample? op) (assoc :activity (activity-snapshot! test))))

      :stop-pre-vote
      (let [before-heal (try
                          {:metrics (liveness/metric-snapshot! test (:nodes test))}
                          (catch Exception e
                            ;; Diagnostic failures must not prevent network recovery.
                            (try
                              (heal! test @roles)
                              (catch Exception cleanup
                                (.addSuppressed e cleanup)))
                            (throw e)))
            observations (atom [])]
        (heal! test @roles)
        (let [recovered? (try
                           (await/until!
                            :pre-vote-catch-up
                            #(let [ms (liveness/metric-snapshot! test (:nodes test))]
                               (swap! observations conj ms)
                               (if (caught-up? ms (:leader @roles))
                                 true
                                 (await/retry! :pre-vote-catch-up {})))
                            {:timeout 10000})
                           (catch Exception e
                             (if (await/condition-timeout? e :pre-vote-catch-up)
                               false
                               (throw e))))]
          (assoc op :value {:status (if recovered? :recovered :incomplete)
                            :before-heal before-heal :recovery-metrics @observations})))))
  (teardown! [_ test] (heal! test @roles))
  nemesis/Reflection
  (fs [_] #{:start-pre-vote :sample-pre-vote :stop-pre-vote}))

(defn- events [history f status]
  (filter #(and (= :nemesis (:process %)) (= f (:f %))
                (= status (get-in % [:value :status]))) history))

(defn stability-checker []
  (reify checker/Checker
    (check [_ test history _]
      (let [start (first (events history :start-pre-vote :installed))
            stop (first (filter #(and (= :nemesis (:process %))
                                      (= :stop-pre-vote (:f %))) history))
            recovered (first (events history :stop-pre-vote :recovered))
            samples (events history :sample-pre-vote :observed)
            final (first (filter :final-sample? samples))
            nodes (set (:nodes test))
            leader (first (:nodes test))
            baseline (get-in start [:value :baseline])
            term (get-in baseline [:metrics leader :current_term])
            bounds? (and start stop recovered final
                         (<= (* observation-seconds 1000000000)
                             (- (:time stop) (:time start)))
                         (< (:time stop) (:time recovered) (:time final)))
            observations (concat [(:metrics baseline) (get-in start [:value :metrics])]
                                 (map #(get-in % [:value :metrics]) samples)
                                 [(get-in recovered [:value :before-heal :metrics])]
                                 (get-in recovered [:value :recovery-metrics]))
            complete? (and (= 5 (count nodes) (count (:nodes test)))
                           start stop
                           (seq (filter #(and (not (:final-sample? %))
                                              (< (:time start) (:time %) (:time stop))) samples))
                           (seq (get-in recovered [:value :recovery-metrics]))
                           (every? #(= nodes (set (keys %))) observations))
            term-errors (for [ms observations [node m] ms
                              :when (not= term (:current_term m))]
                          {:node node :term (:current_term m)})
            role-errors (for [ms observations [node m] ms
                              :when (not= (if (= node leader) "Leader" "Follower") (:state m))]
                          {:node node :state (:state m)})
            five-voters? (every? #(= [(set (map http/node-host nodes))]
                                     (mapv set (get-in % [:membership_config :membership :configs])))
                                 (mapcat vals observations))
            initial-activity (:activity baseline)
            final-activity (get-in final [:value :activity])
            leadership-events? (and (= nodes (set (keys initial-activity)) (set (keys final-activity)))
                                    (every? (fn [node]
                                              (let [before (get initial-activity node)
                                                    after (get final-activity node)]
                                                (and (every? number? (map before [:leader-entries :leader-exits]))
                                                     (= (select-keys before [:leader-entries :leader-exits])
                                                        (select-keys after [:leader-entries :leader-exits])))))
                                            nodes))
            writes (when (and start stop)
                     (liveness/find-successful-writes history (:time start) (:time stop)))
            max-gap (when (and start stop)
                      (apply max (map - (concat (map :time writes) [(:time stop)])
                                      (cons (:time start) (map :time writes)))))
            caught-up? (caught-up? (last (get-in recovered [:value :recovery-metrics])) leader)
            topology? (and (= leader (:leader baseline))
                           (= (topology (:nodes test))
                              (select-keys (:value start) [:leader :core :disconnected :retained :blocked])))]
        {:valid? (boolean (and bounds? complete? topology? (some? term) five-voters? caught-up?
                               (empty? term-errors) (empty? role-errors) leadership-events?
                               (seq writes) max-gap (<= max-gap progress-window-nanos)))
         :term term :leader leader :complete-observations? (boolean complete?)
         :topology-verified? (boolean topology?) :five-voters? five-voters?
         :term-errors (vec term-errors) :role-errors (vec role-errors)
         :leadership-stable? (boolean leadership-events?)
         :successful-writes (count writes) :max-write-gap-ms (some-> max-gap (/ 1e6))
         :observation-complete? (boolean bounds?) :recovered? (boolean (and recovered caught-up?))}))))

(defn package [roles]
  {:nemesis (PreVoteStability. roles) :checker (stability-checker)})

(defn generator [failure-state workload]
  (gen/phases
   (generator/stop-on-harness-failure
    failure-state (gen/nemesis {:type :info :f :start-pre-vote}))
   (gen/shortest-any
    (gen/nemesis
     (generator/stop-on-harness-failure
      failure-state
      (gen/phases
       (gen/time-limit observation-seconds
                       (gen/delay 0.2 (repeat {:type :info :f :sample-pre-vote})))
       ;; time-limit rejects its next scheduled op immediately. Finish the last
       ;; sample interval while clients keep running, ensuring a full 20 seconds.
       (gen/sleep 0.2))))
    (generator/pending-on-harness-failure failure-state (:generator workload)))
   (gen/nemesis {:type :info :f :stop-pre-vote})
   (delay
     (when-not (harness/primary-failure failure-state)
       (generator/stop-on-harness-failure
        failure-state
        (gen/phases (:final-generator workload)
                    (gen/nemesis {:type :info :f :sample-pre-vote :final-sample? true})))))))
