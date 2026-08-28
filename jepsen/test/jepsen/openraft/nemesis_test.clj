(ns jepsen.openraft.nemesis-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.generator :as gen]
            [jepsen.generator.test :as gen-test]
            [jepsen.nemesis :as nemesis]
            [jepsen.openraft [await :as await]
             [cluster :as cluster]
             [db :as db]
             [harness :as harness]
             [nemesis :as openraft-nemesis]
             [worker :as worker]]
            [jepsen.openraft.nemesis.clock :as clock]
            [jepsen.openraft.nemesis.membership :as membership]
            [jepsen.openraft.nemesis.packet :as packet]
            [jepsen.openraft.nemesis.partition :as partition]
            [jepsen.openraft.nemesis.process :as process]))

(def test-config
  {:nodes ["n1" "n2" "n3" "n4" "n5"]})

(deftest schedules-and-retries-skipped-faults
  (doseq [skip-result (concat
                       (map (fn [reason]
                              {:status :skipped
                               :reason reason})
                            [:no-process-target
                             :no-reachable-pause-target
                             :no-safe-packet-target
                             :no-safe-partition-target
                             :no-supported-leader])
                       [{:status :no-supported-leader}
                        :no-supported-leader])]
    (testing (str skip-result)
      (let [faults (#'openraft-nemesis/interval-schedule
                    10
                    3
                    (gen/cycle [{:type :info :f :first}
                                {:type :info :f :second}]))
            generator (gen/shortest-any
                       (gen/nemesis (gen/time-limit 40 faults))
                       (gen/clients
                        (gen/stagger 0.1 (repeat {:f :read}))))
            completion-latency (gen/secs->nanos 2)
            first-attempt? (atom true)
            history (gen-test/simulate
                     generator
                     (fn [_ operation]
                       (let [skip? (and (= :first (:f operation))
                                        (compare-and-set! first-attempt?
                                                          true
                                                          false))]
                         (cond-> (assoc operation :type :ok)
                           skip? (assoc :value skip-result)
                           skip? (update :time + completion-latency)))))
            fault-ops (->> history
                           (filter #(and (= :nemesis (:process %))
                                         (= :info (:type %))))
                           (take 3))
            [_ retry next-fault] fault-ops]
        (is (= [:first :first :second] (mapv :f fault-ops)))
        (is (= (gen/secs->nanos 3) (:time (first fault-ops))))
        (is (<= (gen/secs->nanos 5.5)
                (:time retry)
                (gen/secs->nanos 6.5)))
        (is (<= (+ (:time retry) (gen/secs->nanos 5))
                (:time next-fault)
                (+ (:time retry) (gen/secs->nanos 15))))))))

(deftest cleans-up-all-faults-before-confirming-recovery
  (let [failure-state (harness/failure-state)
        database (db/db {})
        package (openraft-nemesis/compose-packages
                 failure-state
                 [(membership/membership-package
                   database
                   test-config)
                  (partition/partition-package)
                  (process/process-package database)
                  (process/pause-package database)
                  (packet/packet-package database nil)
                  (clock/clock-package)])]
    (is (= [:stop-partition
            :restart-process
            :resume-process
            :stop-packet
            :reset-clock
            :restore-membership
            :await-recovery]
           (mapv :f (:final-generator package))))))

(deftest reports-composed-fault-class-coverage-without-requiring-it
  (let [failure-state (harness/failure-state)
        database (db/db {})
        all-voters (set (:nodes test-config))
        fewer-voters (disj all-voters "n5")
        package (openraft-nemesis/compose-packages
                 failure-state
                 [(partition/partition-package)
                  (process/pause-package database)
                  (packet/packet-package database nil)
                  (clock/clock-package)
                  (membership/membership-package database test-config)])
        check (fn [history]
                (checker/check (:checker package)
                               test-config
                               history
                               {}))
        history [{:f :start-partition
                  :value {:status :installed
                          :mode :leader-in-majority}}
                 {:f :stop-partition
                  :value {:status :installed}}
                 {:f :pause-process
                  :value {:status :installed
                          :mode :random
                          :leader "n1"
                          :nodes ["n2"]
                          :voter-configs [all-voters]
                          :target-category :one
                          :pause-results {"n2" :paused}}}
                 {:f :resume-process
                  :value {:status :installed
                          :paused nil
                          :resumed (:nodes test-config)
                          :resume-results
                          (zipmap (:nodes test-config) (repeat :resumed))}}
                 {:f :start-packet
                  :value {:status :installed
                          :mode :slow
                          :target-role :leader-included}}
                 {:f :stop-packet
                  :value {:status :installed
                          :mode :slow}}
                 {:f :bump-clock
                  :value {:status :installed
                          :mode :bump}}
                 {:f :reset-clock
                  :value {:status :installed}}
                 {:f :shrink
                  :value {:status :installed
                          :change :shrink
                          :node "n5"
                          :leader "n1"
                          :before all-voters
                          :after fewer-voters}}
                 {:f :restore-membership
                  :value {:status :installed
                          :leader "n1"
                          :voters all-voters}}
                 {:f :await-recovery
                  :value {:status :installed
                          :leader "n1"}}]]
    (testing "one successful mode covers each fault class in chaos"
      (let [result (check history)]
        (is (:valid? result))
        (is (true? (get-in result [:partition :fault-class-executed?])))
        (is (true? (get-in result [:pause :fault-class-executed?])))
        (is (true? (get-in result [:packet :fault-class-executed?])))
        (is (true? (get-in result [:clock :fault-class-executed?])))
        (is (true? (get-in result
                           [:membership :fault-class-executed?])))))

    (testing "a skipped fault is reported, not a composed-run failure"
      (let [result (check (mapv #(if (= :pause-process (:f %))
                                   (assoc % :value
                                          {:status :skipped
                                           :reason :no-supported-leader})
                                   %)
                                history))]
        (is (:valid? result))
        (is (false? (get-in result [:pause :fault-class-executed?])))
        (is (= [:random] (get-in result [:pause :missing-modes])))))

    (testing "an installed outcome the class cannot read stays a failure"
      (let [result (check (mapv #(if (= :start-partition (:f %))
                                   (assoc % :value {:status :installed})
                                   %)
                                history))]
        (is (false? (:valid? result)))
        (is (false? (get-in result [:partition :fault-class-executed?])))
        (is (= 1 (get-in result [:partition :unrecognized-installs])))
        (is (= :intact (get-in result [:partition :cluster-state])))))))

(defn- teardown-package [package-name events throwable]
  {:name package-name
   :interval 1
   :nemesis
   (reify nemesis/Nemesis
     (setup! [this _test]
       this)

     (invoke! [_ _test op]
       op)

     (teardown! [this _test]
       (swap! events conj package-name)
       (when throwable
         (throw throwable))
       this)

     nemesis/Reflection
     (fs [_]
       #{package-name}))
   :generator {:type :info
               :f package-name}
   :final-generator nil
   :checker (reify checker/Checker
              (check [_ _test _history _opts]
                {:valid? true}))
   :perf #{}})

(deftest composed-teardown-records-failures-and-continues
  (let [failure-state (harness/failure-state)
        events (atom [])
        first-error (RuntimeException. "partition cleanup failed")
        second-error (RuntimeException. "pause cleanup failed")
        package (openraft-nemesis/compose-packages
                 failure-state
                 [(teardown-package :partition events first-error)
                  (teardown-package :pause events second-error)
                  (teardown-package :process events nil)])
        subject (nemesis/setup! (:nemesis package) test-config)]
    (nemesis/teardown! subject test-config)
    (swap! events conj :analysis)
    (is (= [:partition :pause :process :analysis] @events))
    (let [primary (harness/primary-failure failure-state)
          [secondary] (harness/secondary-failures failure-state)]
      (is (= :nemesis (:source primary)))
      (is (= {:phase :teardown
              :component :partition
              :nodes (:nodes test-config)}
             (:context primary)))
      (is (identical? first-error (:throwable primary)))
      (is (= :nemesis (:source secondary)))
      (is (= {:phase :teardown
              :component :pause
              :nodes (:nodes test-config)}
             (:context secondary)))
      (is (identical? second-error (:throwable secondary))))))

(deftest composed-teardown-preserves-clean-behavior
  (let [failure-state (harness/failure-state)
        events (atom [])
        package (openraft-nemesis/compose-packages
                 failure-state
                 [(teardown-package :partition events nil)
                  (teardown-package :process events nil)])
        subject (nemesis/setup! (:nemesis package) test-config)]
    (nemesis/teardown! subject test-config)
    (is (= [:partition :process] @events))
    (is (nil? (harness/primary-failure failure-state)))
    (is (empty? (harness/secondary-failures failure-state)))))

(deftest composed-teardown-propagates-interruption
  (Thread/interrupted)
  (let [failure-state (harness/failure-state)
        events (atom [])
        interruption (InterruptedException. "stop teardown")
        package (openraft-nemesis/compose-packages
                 failure-state
                 [(teardown-package :partition events interruption)
                  (teardown-package :process events nil)])
        subject (nemesis/setup! (:nemesis package) test-config)]
    (try
      (let [[thrown interrupted?]
            (try
              (nemesis/teardown! subject test-config)
              [nil (.isInterrupted (Thread/currentThread))]
              (catch Throwable throwable
                [throwable (.isInterrupted (Thread/currentThread))]))]
        (is (identical? interruption thrown))
        (is interrupted?)
        (is (= [:partition] @events))
        (is (nil? (harness/primary-failure failure-state)))
        (is (empty? (harness/secondary-failures failure-state))))
      (finally
        (Thread/interrupted)))))

(defn- condition-timeout [condition]
  (try
    (await/until! condition
                  #(await/retry! condition {})
                  {:timeout 0
                   :retry-interval 0})
    nil
    (catch Exception e
      e)))

(deftest classifies-recovery-outcomes
  (let [op {:type :info
            :process :nemesis
            :f :await-recovery}
        subject (openraft-nemesis/->RecoveryNemesis)]
    (testing "confirmed recovery is installed"
      (with-redefs [cluster/await-ready! (constantly {:leader "n2"})]
        (is (= {:status :installed
                :leader "n2"}
               (:value (nemesis/invoke! subject test-config op))))))

    (testing "a modeled readiness timeout is indeterminate"
      (let [error (condition-timeout :cluster-ready)]
        (with-redefs [cluster/await-ready! (fn [_test] (throw error))]
          (let [value (:value (nemesis/invoke! subject test-config op))]
            (is (= :indeterminate (:status value)))
            (is (= :recovery-timeout (:reason value)))))))

    (testing "an unexpected recovery exception takes the Harness path"
      (let [failure-state (harness/failure-state)
            error (RuntimeException. "recovery bug")
            wrapped (worker/wrap-nemesis failure-state subject)
            setup-subject (nemesis/setup! wrapped test-config)
            thrown (with-redefs [cluster/await-ready! (fn [_test]
                                                        (throw error))]
                     (try
                       (nemesis/invoke! setup-subject test-config op)
                       nil
                       (catch Exception e
                         e)))]
        (is (identical? error thrown))
        (is (identical? error
                        (:throwable
                         (harness/primary-failure failure-state))))))

    (testing "an unrelated timeout tag takes the Harness path"
      (let [failure-state (harness/failure-state)
            error (ex-info "unknown timeout" {:type :timeout})
            wrapped (worker/wrap-nemesis failure-state subject)
            setup-subject (nemesis/setup! wrapped test-config)
            thrown (with-redefs [cluster/await-ready! (fn [_test]
                                                        (throw error))]
                     (try
                       (nemesis/invoke! setup-subject test-config op)
                       nil
                       (catch Exception e
                         e)))]
        (is (identical? error thrown))
        (is (identical? error
                        (:throwable
                         (harness/primary-failure failure-state))))))

    (testing "interruption takes priority over timeout classification"
      (doseq [[label make-error]
              [[:interrupted #(InterruptedException. "interrupted")]
               [:interrupted-io
                #(java.io.InterruptedIOException. "interrupted")]
               [:closed-by-interrupt
                #(java.nio.channels.ClosedByInterruptException.)]
               [:wrapped
                #(ex-info "interrupted"
                          {:kind :interrupted
                           :type :timeout})]]]
        (Thread/interrupted)
        (try
          (let [failure-state (harness/failure-state)
                error (make-error)
                wrapped (worker/wrap-nemesis failure-state subject)
                setup-subject (nemesis/setup! wrapped test-config)
                [thrown interrupted?]
                (with-redefs [cluster/await-ready!
                              (fn [_test] (throw error))]
                  (try
                    (nemesis/invoke! setup-subject test-config op)
                    [nil (.isInterrupted (Thread/currentThread))]
                    (catch Exception e
                      [e (.isInterrupted (Thread/currentThread))])))]
            (is (identical? error thrown) (name label))
            (is interrupted? (name label))
            (is (nil? (harness/primary-failure failure-state)) (name label)))
          (finally
            (Thread/interrupted)))))))
