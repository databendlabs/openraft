(ns jepsen.openraft.liveness
  "Fixed partial-connectivity liveness test, separate from random Chaos."
  (:require [jepsen [checker :as checker]
             [control :as c]
             [generator :as gen]
             [nemesis :as nemesis]]
            [jepsen.openraft.await :as await]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.cluster :as cluster]))

(def observation-seconds 10)
(def deadline-nanos 1500000000)

;; Installed topology: five voters, quorum = 3. All links are bidirectional.
;;
;;       a -------- b       isolated
;;        \        /
;;         \      /
;;          bridge
;;             |
;;          leader
;;
;; Cut links: leader --X-- a, leader --X-- b.
;; The isolated voter has no Raft links; all five processes remain running.
;; a and b can campaign to replace the old leader. With the bridge, they form
;; a connected quorum. The bridge still receives the old leader's heartbeats.
(defn topology [nodes leader]
  (when-not (and (= 5 (count nodes) (count (set nodes)))
                 (some #{leader} nodes))
    (throw (ex-info "Partial-network liveness requires five distinct nodes including the leader" {})))
  (let [[bridge a b isolated] (remove #{leader} (sort nodes))]
    {:leader leader :bridge bridge :majority [bridge a b]
     :cut [a b] :isolated isolated}))

(defn- metric-snapshot! [test nodes]
  (into {} (map (fn [node] [node (cluster/node-metrics! test node)]) nodes)))

(defn- cut! [test leader nodes]
  (c/on leader
        (c/su
         (doseq [node nodes
                 :let [ip (.getHostAddress (java.net.InetAddress/getByName
                                            (http/node-host node)))]]
           (c/exec :iptables :-A :OR_LIVE :-s ip :-p :tcp
                   :-m :multiport :--ports (:raft-port test 22001) :-j :DROP)
           (c/exec :iptables :-A :OR_LIVE :-d ip :-p :tcp
                   :-m :multiport :--ports (:raft-port test 22001) :-j :DROP)))))

(defn- heal! [roles]
  (doseq [node (keep #(get @roles %) [:leader :isolated])]
    (c/on node
          (c/su
           (c/exec :bash :-c
                   (str "if iptables -S OR_LIVE >/dev/null 2>&1; then "
                        "while iptables -C INPUT -j OR_LIVE 2>/dev/null; do "
                        "iptables -D INPUT -j OR_LIVE || exit; done; "
                        "while iptables -C OUTPUT -j OR_LIVE 2>/dev/null; do "
                        "iptables -D OUTPUT -j OR_LIVE || exit; done; "
                        "iptables -F OR_LIVE && iptables -X OR_LIVE; fi"))))))

(defrecord PartialNetwork [roles]
  nemesis/Nemesis
  (setup! [this _] this)
  (invoke! [_ test op]
    (case (:f op)
      :start-liveness
      (let [{:keys [leader]} (cluster/await-ready! test)
            {:keys [cut isolated] :as plan} (topology (:nodes test) leader)]
        ;; No workload runs during this phase: equal logs exclude freshness as
        ;; an independent reason to reject candidates after partitioning.
        (await/until!
         :equal-logs
         #(let [ms (metric-snapshot! test (:nodes test))]
            (if (and (apply = (map :last_applied (vals ms)))
                     (every? (fn [m] (= (:last_log_index m)
                                        (get-in m [:last_applied :index])))
                             (vals ms)))
              ms
              (await/retry! :equal-logs {})))
         {:timeout 10000})
        (reset! roles plan)
        (doseq [node [leader isolated]]
          (c/on node
                (c/su
                 (c/exec :iptables :-N :OR_LIVE)
                 (c/exec :iptables :-I :INPUT :-j :OR_LIVE)
                 (c/exec :iptables :-I :OUTPUT :-j :OR_LIVE))))
        (cut! test isolated (remove #{isolated} (:nodes test)))
        (cut! test leader cut)
        (assoc op :value (assoc plan :status :installed)))

      :sample-liveness
      (assoc op :value {:status :observed
                        :metrics (metric-snapshot! test (:nodes test))})

      :stop-liveness
      (do (heal! roles)
          (assoc op :value (assoc (cluster/await-ready! test) :status :recovered)))))
  (teardown! [_ _] (heal! roles))
  nemesis/Reflection
  (fs [_] #{:start-liveness :sample-liveness :stop-liveness}))

(defn- matching-nemesis-events [history f status]
  (filter #(and (= :nemesis (:process %)) (= f (:f %))
                (= status (get-in % [:value :status]))) history))

(defn- find-successful-writes [history start end]
  (:writes
   (reduce (fn [{:keys [pending] :as acc} op]
             (cond
               (= :invoke (:type op)) (assoc-in acc [:pending (:process op)] op)
               (#{:ok :fail :info} (:type op))
               (let [invocation (get pending (:process op))]
                 (cond-> (update acc :pending dissoc (:process op))
                   (and invocation (= :ok (:type op)) (= :write (:f op))
                        (<= start (:time invocation) (:time op))
                        (< (:time op) end))
                   (update :writes conj op)))
               :else acc))
           {:pending {} :writes []} history)))

(defn liveness-checker []
  (reify checker/Checker
    (check [_ test history _]
      (let [start (first (matching-nemesis-events history :start-liveness :installed))
            stop (first (filter #(and (= :nemesis (:process %))
                                      (= :stop-liveness (:f %))) history))
            recovered (first (matching-nemesis-events history :stop-liveness :recovered))
            bounds? (and start stop recovered
                         (< (:time start) (:time stop)))
            samples (matching-nemesis-events history :sample-liveness :observed)
            majority (get-in start [:value :majority])
            expected-nodes (set (conj majority (get-in start [:value :leader])
                                      (get-in start [:value :isolated])))
            complete-samples? (and (seq samples) (= 5 (count expected-nodes))
                                   (every? #(= expected-nodes
                                               (set (keys (get-in % [:value :metrics]))))
                                           samples))
            phase (fn [from to]
                    (let [writes (find-successful-writes history from to)]
                      {:successful-writes (count writes)
                       :first-success-ms (some-> (first writes) :time (- from) (/ 1e6))
                       :within-deadline? (boolean
                                          (some #(< (:time %) (+ from deadline-nanos))
                                                writes))}))
            original (when bounds? (phase (:time start) (:time stop)))
            configs (for [op (matching-nemesis-events history :sample-liveness :observed)
                          m (vals (get-in op [:value :metrics]))]
                      (get-in m [:membership_config :membership :configs]))
            five-voters? (and (= 5 (count (:nodes test)) (count (set (:nodes test))))
                              (seq configs)
                              (every? #(= [(set (map http/node-host (:nodes test)))]
                                          (mapv set %)) configs))]
        {:valid? (boolean (and bounds? five-voters? complete-samples?
                               (:within-deadline? original)))
         :original-topology original
         :five-voters? (boolean five-voters?) :recovered? (boolean recovered)}))))

(defn package [roles]
  {:nemesis (PartialNetwork. roles)
   :checker (liveness-checker)})

(defn generator [workload]
  (let [samples #(gen/time-limit observation-seconds
                                 (gen/stagger 0.2
                                              (repeat {:type :info :f :sample-liveness})))]
    (gen/phases
     (gen/nemesis {:type :info :f :start-liveness})
     (gen/shortest-any
      (gen/nemesis (samples))
      ;; Let the old leader's initial quorum lease expire before the normal
      ;; client follows any redirect to it. Otherwise an early uncommitted
      ;; write makes the bridge's log newer and confounds the lease experiment.
      ;; The liveness deadline still starts at installation, not after this wait.
      (gen/phases (gen/sleep 0.6) (:generator workload)))
     (gen/nemesis {:type :info :f :stop-liveness})
     (:final-generator workload))))
