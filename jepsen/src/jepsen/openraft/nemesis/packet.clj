(ns jepsen.openraft.nemesis.packet
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.nemesis.outcome :as outcome]
            [jepsen.openraft.quorum :as quorum]))

(def packet-seconds 10)
(def packet-modes #{:slow :flaky})
(def required-target-roles #{:leader-included :leader-excluded})

(def ^:private packet-behaviors
  {:slow {:delay {:time :300ms
                  :jitter :50ms
                  :correlation :25%
                  :distribution :normal}}
   :flaky {:loss {}}})

(defn- packet-targets [configs leader target-role]
  (let [voters (quorum/voter-set configs)]
    (when-not (contains? voters leader)
      (throw (ex-info "The current leader is not a voter"
                      {:leader leader
                       :configs configs})))
    (let [candidates (case target-role
                       :leader-included
                       (filter #(contains? % leader)
                               (quorum/fault-sets configs))

                       :leader-excluded
                       (remove #(contains? % leader)
                               (quorum/fault-sets configs))

                       (throw (ex-info "Unknown packet target role"
                                       {:target-role target-role})))]
      (first (random/shuffle candidates)))))

(defn- invoke-delegate! [delegate test op]
  (try
    (nemesis/invoke! delegate test op)
    (catch Exception e
      (when (interruption/interruption? e)
        (.interrupt (Thread/currentThread)))
      (throw e))))

(defrecord PacketNemesis [delegate packet-mode]
  nemesis/Nemesis
  (setup! [_ test]
    (PacketNemesis. (nemesis/setup! delegate test) packet-mode))

  (invoke! [_ test op]
    (case (:f op)
      :start-packet
      (if-let [{:keys [leader] :as status}
               (cluster/membership-status test)]
        (let [configs (cluster/voter-configs test status)
              request (:value op)
              mode (or (:mode request) packet-mode)
              target-role (or (:target-role request) request)]
          (when-not (packet-modes mode)
            (throw (ex-info "Unknown packet mode" {:mode mode})))
          (if-let [targets (packet-targets configs leader target-role)]
            (let [targets (vec (filter targets (:nodes test)))
                  behavior (get packet-behaviors mode)]
              (info "Degrading OpenRaft packet delivery"
                    {:mode mode
                     :target-role target-role
                     :leader leader
                     :targets targets
                     :behavior behavior})
              (invoke-delegate! delegate
                                test
                                (assoc op
                                       :value [targets behavior]))
              (assoc op
                     :value (outcome/installed
                             {:mode mode
                              :target-role target-role
                              :leader leader
                              :voter-configs configs
                              :targets targets
                              :behavior behavior})))
            (do
              (info "Skipping packet degradation without a safe target"
                    {:mode mode
                     :target-role target-role
                     :leader leader
                     :voter-configs configs})
              (assoc op
                     :value (outcome/skipped
                             :no-safe-packet-target
                             {:mode mode
                              :target-role target-role
                              :leader leader
                              :voter-configs configs})))))
        (do
          (info "Skipping packet degradation without a supported leader")
          (assoc op
                 :value (outcome/skipped
                         :no-supported-leader
                         {:mode (or (:mode (:value op)) packet-mode)
                          :target-role (or (:target-role (:value op))
                                           (:value op))}))))

      :stop-packet
      (do
        (info "Restoring reliable OpenRaft packet delivery")
        (invoke-delegate! delegate test (assoc op :value nil))
        (assoc op :value (outcome/installed {:mode packet-mode})))))

  (teardown! [_ test]
    (try
      (nemesis/teardown! delegate test)
      (catch Exception e
        (when (interruption/interruption? e)
          (.interrupt (Thread/currentThread)))
        (throw e))))

  nemesis/Reflection
  (fs [_]
    #{:start-packet :stop-packet}))

(defn packet-nemesis [database packet-mode]
  (when-not (or (nil? packet-mode)
                (packet-modes packet-mode))
    (throw (ex-info "Unknown packet mode" {:mode packet-mode})))
  (PacketNemesis. (combined/packet-nemesis database) packet-mode))

(defn- packet-start-installed? [value]
  (and (= :installed (:status value))
       (packet-modes (:mode value))
       (required-target-roles (:target-role value))))

(defn- packet-stop-installed? [value]
  (= :installed (:status value)))

(defn- recovery-installed? [value]
  (and (= :installed (:status value))
       (:leader value)))

(defn- select-packet-mode [packet-mode]
  (or packet-mode
      (first (random/shuffle packet-modes))))

(defn- packet-generator [packet-mode]
  (let [start (fn [target-role]
                (fn [_test _context]
                  {:type :info
                   :f :start-packet
                   :value {:mode (select-packet-mode packet-mode)
                           :target-role target-role}}))
        stop {:type :info
              :f :stop-packet}]
    (gen/cycle
     (gen/phases
      (gen/once (start :leader-included))
      stop
      (gen/once (start :leader-excluded))
      stop))))

(defn- next-cluster-state [state op]
  (let [status (get-in op [:value :status])
        operation-error? (boolean (or (:error op)
                                      (:exception op)
                                      (= :indeterminate status)))]
    (case (:f op)
      :start-packet
      (cond
        operation-error? :unknown
        (packet-start-installed? (:value op)) :recovery-pending
        :else state)

      :stop-packet
      (cond
        operation-error? :unknown
        (packet-stop-installed? (:value op)) :recovery-pending
        :else state)

      :await-recovery
      (cond
        operation-error? (if (= :recovery-pending state)
                           :recovery-pending
                           :unknown)
        (= :unknown state) :unknown
        (recovery-installed? (:value op)) :intact
        :else state)

      state)))

(defn- coverage-checker [packet-mode]
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [observed-roles (->> history
                                (filter #(= :start-packet (:f %)))
                                (filter #(packet-start-installed? (:value %)))
                                (filter #(or (nil? packet-mode)
                                             (= packet-mode
                                                (get-in % [:value :mode]))))
                                (keep #(get-in % [:value :target-role]))
                                set)
            missing-roles (remove observed-roles required-target-roles)
            cluster-state (reduce next-cluster-state :intact history)
            valid? (cond
                     (seq missing-roles) false
                     (= :intact cluster-state) true
                     (= :recovery-pending cluster-state) false
                     :else :unknown)]
        {:valid? valid?
         :mode (or packet-mode :mixed)
         :observed-modes (vec (sort observed-roles))
         :missing-modes (vec (sort missing-roles))
         :cluster-state cluster-state}))))

(defn packet-package [database packet-mode]
  {:name :packet
   :interval packet-seconds
   :nemesis (packet-nemesis database packet-mode)
   :generator (packet-generator packet-mode)
   :final-generator {:type :info
                     :f :stop-packet}
   :checker (openraft-checker/reject-checker-exceptions
             (coverage-checker packet-mode))})
